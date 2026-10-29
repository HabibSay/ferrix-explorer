fn main() {
    // Add the workspace root to the Slint include search path so that
    // `import { Strings } from "resources/en-US.slint"` resolves correctly.
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // manifest_dir is  <workspace>/crates/fx-ui
    let workspace_root = manifest_dir
        .parent() // crates/
        .and_then(|p| p.parent()) // workspace root
        .expect("workspace root must be two levels above CARGO_MANIFEST_DIR");

    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![workspace_root.to_path_buf()]);

    slint_build::compile_with_config("src/ui.slint", config).unwrap();
}
