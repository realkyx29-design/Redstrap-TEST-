//! Windows-only: embed the application icon and manifest.
//!
//! The `.rc` is generated with absolute paths so it compiles identically
//! under `rc.exe` and `windres`, regardless of working-directory semantics.

fn main() {
    #[cfg(windows)]
    {
        use std::path::PathBuf;

        let manifest_dir =
            PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| String::from(".")));
        let src = manifest_dir.join("build").join("windows");
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_else(|_| String::from(".")))
            .join("redstrap.rc");

        // Forward slashes keep the RC valid on every toolchain.
        let slash = |p: PathBuf| p.to_string_lossy().replace('\\', "/");
        let rc = format!(
            "#define RT_MANIFEST 24\n1 RT_MANIFEST \"{}\"\n101 ICON \"{}\"\n",
            slash(src.join("app.manifest")),
            slash(src.join("icon.ico")),
        );
        std::fs::write(&out, rc).expect("write generated rc file");
        embed_resource::compile(&out, embed_resource::NONE);

        println!("cargo:rerun-if-changed=build/windows/app.manifest");
        println!("cargo:rerun-if-changed=build/windows/icon.ico");
    }
}
