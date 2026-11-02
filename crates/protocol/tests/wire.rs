//! VC-3: the wire format of `Opened` and `Corpus` is pinned; any snapshot change bumps `PROTOCOL`.
use std::{path::Path, process::Command};

use spec_viewer_core::{ParseConfig, docs, document::Documents};
use spec_viewer_protocol::{Corpus, Opened, PROTOCOL};

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap().join("corpus");
    std::fs::create_dir_all(root.join("spec")).unwrap();
    std::fs::write(
        root.join("spec/dialect.spec.mdx"),
        include_str!("../../../fixtures/dialect.spec.mdx"),
    )
    .unwrap();
    let status = Command::new("git").arg("-C").arg(&root).args(["init", "-q"]).status().unwrap();
    assert!(status.success());
    (directory, root)
}

fn redacted(json: &str, root: &Path) -> String {
    json.replace(&root.display().to_string(), "<root>")
}

#[test]
fn vc_3_opened_and_corpus_match_their_snapshots_and_round_trip() {
    assert_eq!(PROTOCOL, 1, "a protocol bump re-pins the snapshots");
    let (_directory, root) = fixture();
    let config = ParseConfig::default();
    let opened = Documents::new(config.clone()).open(&root, "spec/dialect.spec.mdx").unwrap();
    let json = serde_json::to_string_pretty(&opened).unwrap();
    insta::assert_snapshot!("vc_3_opened", redacted(&json, &root));
    assert_eq!(serde_json::from_str::<Opened>(&json).unwrap(), opened);

    let corpus = docs(&root, "corpus", &config);
    let json = serde_json::to_string_pretty(&corpus).unwrap();
    insta::assert_snapshot!("vc_3_corpus", redacted(&json, &root));
    assert_eq!(serde_json::from_str::<Corpus>(&json).unwrap(), corpus);
}
