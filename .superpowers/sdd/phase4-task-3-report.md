# Phase IV Task 3 Report — FLCM Context and Silence Warning

## Baseline

- Branch: `remotive-integration`
- BASE before implementation: `190363f9b2f12403f5788bfdd8051dda7396e407`

## Implemented

- Added `FlcmContext` with independent left/right `ObservedBool` lamp status and `silent`.
- Added a tell-back `FlcmActor`, preserving the existing SCCM/BCM startup barrier contract.
- Projected `LeftLowBeamStatus` and `RightLowBeamStatus` ingress into FLCM zone turns.
- Added a powered 500 ms liveness deadline refreshed by either status observation.
- Added structured `DiagnosticKind::FlcmLampFault` Warning emission on silence or Fail.
- Latched each unhealthy interval so repeated ticks/fault facts do not repeat the Warning.
- Added Info clear emission only after both statuses are Ok and traffic is no longer silent.
- Reset FLCM context, liveness, duplicate streaks, and warning latch across power cycles.
- Kept schema version unchanged; v1 temporarily projects FLCM diagnostics as existing `Text`.
- Kept bridge subscriptions unchanged. TUI received only the exhaustive-match compile arm.

## TDD Evidence

1. Pure contract target initially failed because `FlcmContext`, `FlcmMessage`, and
   `FlcmLampFault` did not exist.
2. Runtime contracts then failed because low-beam status ingress was rejected.
3. The power-cycle contract failed because duplicate streaks survived restart.
4. Each failure was followed by the minimal implementation and a passing rerun.

## Verification

- `cargo test -p common --test flcm_observation_contract`: 10 passed.
- `cargo test -p common diagnostic`: passed.
- `cargo test -p common`: 293 unit + 10 FLCM integration tests passed.
- `cargo check --workspace --all-targets`: passed.
- `git diff --check`: passed.

## Self-review

- Fixed an initial design regression where adding FLCM to `ALL_ASSEMBLIES` changed the
  locked SCCM/BCM startup topology. FLCM now powers alongside that barrier.
- Fixed restart handling so the first post-restart value is never discarded as a duplicate.
- Warning clearing is intentionally stricter than “no current Fail”: both sides must be
  explicitly Ok and liveness restored.

## Deferred / Concerns

- Task 4 still needs first-class FLCM fields/events/diagnostics in the published schema;
  Task 3 deliberately does not bump the schema version.
- Task 5 must subscribe the bridge to FLCM signals.
- Task 6 must provide attended TUI formatting; the native variant is currently hidden there,
  while the unchanged v1 wire path carries an ASCII `Text` fallback.
- Workspace check retains one pre-existing TUI unused-import warning.
