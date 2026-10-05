mod google_app_build;

fn main() {
    google_app_build::stage().unwrap_or_else(|message| panic!("{message}"));
    println!("cargo:rerun-if-env-changed=RIVIU_DEFAULT_AGENT_MODE");
    if let Ok(mode) = std::env::var("RIVIU_DEFAULT_AGENT_MODE") {
        println!("cargo:rustc-env=RIVIU_DEFAULT_AGENT_MODE={mode}");
    }
    // Tauri exposes custom-protocol through DEP_TAURI_DEV, not a crate-local feature.
    // Plain cargo check and devUrl builds must not require a frontend dist or Node.
    println!("cargo:rerun-if-env-changed=DEP_TAURI_DEV");
    println!("cargo:rerun-if-env-changed=TAURI_CONFIG");
    if !tauri_build::is_dev() {
        let desktop = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .parent().unwrap().to_path_buf();
        let check = std::process::Command::new("node")
            .arg("../../scripts/frontend_provenance.mjs")
            .arg("verify-cargo")
            .current_dir(desktop)
            .output()
            .expect("frontend provenance needs Node; run npm run build before embedding assets");
        print!("{}", String::from_utf8_lossy(&check.stdout));
        assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stderr));
    }
    tauri_build::build()
}
