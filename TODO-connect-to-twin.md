# Moved — process split complete

Content from this plan is **merged into the livedocs**:

- **Target topology (Gateway vs Dashboard):** [`docs/ARCHITECTURE-OVERVIEW.md`](docs/ARCHITECTURE-OVERVIEW.md) §1
- **Process split (Done):** Gateway/Dashboard split — [`docs/ARCHITECTURE-OVERVIEW.md`](docs/ARCHITECTURE-OVERVIEW.md) §2
- **Design decisions:** [`docs/DESIGN.md`](docs/DESIGN.md)
- **Remotive observation path:** [`docs/DESIGN-remotive-observation.md`](docs/DESIGN-remotive-observation.md)
- **TwinRuntimeBuilder / channel ownership:** still valid; see `crates/gateway/src/gateway_runtime.rs`

Live observation: exclusive **UDS** or **peer Zenoh** (Zenoh live link Done). File tee always on.

*Redirect stub — updated for Remotive observation docs*
