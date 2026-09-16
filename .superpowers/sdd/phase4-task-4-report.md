# Task 4 Report: Observation schema v7 + publish facade

## Status

Complete.

## Baseline

- Branch: `remotive-integration`
- BASE: `f34fc76017a40c3253c70144babbf9ae21b892a4`

## Changes

- Bumped `CURRENT_SCHEMA_VERSION` to 7.
- Added `PublishedFlcmContext` to the transition facade and `PublishedVehicleContext`.
- Added `FlcmContextV1` with tri-state left/right low-beam status and `silent`.
- New v7 ledger rows emit `flcm`; v1-v6 ledgers without it default to Unknown/Unknown/false.
- Added a dedicated `flcm_lamp_fault` diagnostic wire variant and live round-trip mapping.
- Added v7 golden files and compatibility, projection, and round-trip coverage.
- Updated gateway/TUI test fixtures only as required to compile with the new facade field.
- Did not change the Remotive bridge subscription list.

## Verification

- `cargo test -p observation` — PASS
- `cargo test -p common` — PASS (293 unit tests + 11 FLCM integration tests)
- `cargo test -p common observation_records::transition::published_projection_tests` — PASS
- `cargo check --workspace --all-targets` — PASS
- `git diff --check` — PASS

## Concerns

- Workspace check retains one pre-existing unused-import warning in
  `crates/tui_dashboard/src/view/driver.rs`.
