mod support;

use std::path::{Path, PathBuf};

use observation::RunWriter;
use support::{RUN_ID, fixed_run_metadata, sample_diagnostic, sample_ledger};

fn golden_run_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/golden/v4")
        .join(RUN_ID)
}

#[test]
fn writer_output_matches_committed_golden_run_byte_for_byte() {
    let temp = tempfile::tempdir().unwrap();
    let metadata = fixed_run_metadata();
    let mut writer = RunWriter::create(temp.path(), metadata).unwrap();
    writer.record_diagnostic(&sample_diagnostic()).unwrap();
    writer.record_ledger(&sample_ledger()).unwrap();
    writer.finish().unwrap();

    let actual = temp.path().join(RUN_ID);
    let expected = golden_run_dir();

    if std::env::var_os("UPDATE_OBSERVATION_GOLDEN").is_some() {
        for name in ["manifest.json", "diagnostic.jsonl", "ledger.jsonl"] {
            std::fs::copy(actual.join(name), expected.join(name)).unwrap();
        }
        return;
    }

    for name in ["manifest.json", "diagnostic.jsonl", "ledger.jsonl"] {
        assert_eq!(
            std::fs::read(actual.join(name)).unwrap(),
            std::fs::read(expected.join(name)).unwrap(),
            "golden mismatch in {name}"
        );
    }
}
