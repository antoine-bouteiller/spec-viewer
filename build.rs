use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create embedded Mermaid directory");
    for entry in fs::read_dir(source).expect("read Mermaid assets") {
        let entry = entry.expect("read Mermaid entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("stat Mermaid entry").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy Mermaid asset");
        }
    }
}

fn main() {
    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../..");
    let mermaid = root.join("node_modules/mermaid");
    assert!(
        mermaid.join("dist/mermaid.esm.min.mjs").is_file(),
        "plan-viewer assets require node_modules; run bun install at the repository root"
    );
    for path in [root.join("src/app"), root.join("src/index.html"), mermaid.clone()] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let output = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("assets");
    if output.exists() {
        fs::remove_dir_all(&output).expect("remove previous frontend bundle");
    }
    let result = Command::new("bun")
        .current_dir(&root)
        .args(["build", "src/index.html", "--production", "--public-path=/", "--outdir"])
        .arg(&output)
        .output()
        .expect("plan-viewer assets require Bun on PATH; install Bun before building");
    assert!(
        result.status.success(),
        "plan-viewer frontend bundle failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let destination = output.join("mermaid/dist");
    fs::create_dir_all(&destination).expect("create embedded Mermaid directory");
    fs::copy(mermaid.join("dist/mermaid.esm.min.mjs"), destination.join("mermaid.esm.min.mjs")).expect("copy Mermaid entrypoint");
    copy_tree(
        &mermaid.join("dist/chunks/mermaid.esm.min"),
        &destination.join("chunks/mermaid.esm.min"),
    );
}
