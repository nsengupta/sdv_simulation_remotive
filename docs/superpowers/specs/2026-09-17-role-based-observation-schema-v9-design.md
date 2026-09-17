# Role-based observation schema v9

**Date:** 2026-09-17  
**Branch baseline:** `remotive-integration` at `68d34bf`  
**Status:** Design approved; awaiting implementation plan  
**Decision:** Use role-based Rust DTO names, one uniform writer schema version, and backward-compatible readers.

## Goal

Make the observation boundary unambiguous:

- `common::fsm::FsmEvent` remains the application's only authoritative runtime FSM event.
- Persisted DTOs are named for their role rather than for the first schema version in which their shape appeared.
- Every newly written manifest, diagnostic envelope, and ledger envelope declares schema version 9.
- The upgraded reader loads schema versions 1 through 9, preserving historical versions 1–8.
- Assembly readiness is recorded truthfully instead of being mislabeled as a timer tick.
- The design and tests leave enough migration evidence to reproduce the change later in the original Twin project.

## Problems

### Misleading version names

`observation::schema::v1` contains the current schema-v8 writer DTOs, including types such as `FsmEventV1` with variants introduced after version 1. The suffix and module name therefore do not describe an immutable v1 schema.

### Multiple representations look authoritative

The runtime event, published projection, and persisted event have similar names:

1. `common::fsm::FsmEvent` — runtime behavior.
2. `common::facade::PublishedFsmEvent` — process-independent observation projection.
3. `observation::schema::v1::FsmEventV1` — JSON ledger representation.

Only the first is authoritative for FSM behavior. The other two are boundary representations. Their names must communicate that distinction.

### False event projection

`FsmEvent::AssemblyZoneReady(_)` currently projects to `PublishedFsmEvent::TimerTick`. This preserves a row but records an event that did not occur. Existing tests exercise assembly readiness and ledger sequencing, but no unit test asserts truthful projection for this variant.

### Inconsistent documentation

Reader documentation still mentions support for versions 1–5 even though the current code supports versions 1–8. Version support must be stated once in constants and repeated accurately in boundary documentation.

## Chosen approach

Use a role-based current-schema module. Rename `schema/v1.rs` to `schema/entries.rs` and expose it as `observation::schema::entries`.

Do not create copies of every DTO under `v1` through `v9`. The existing JSON shapes are additive and one current DTO family can deserialize versions 1–9. Historical compatibility remains an explicit reader and test responsibility.

Rust type renames do not alter serialized JSON. Schema 9 changes the wire format only by adding the truthful `assembly_zone_ready` event and its assembly identifier.

## Naming contract

Use idiomatic Rust acronym casing: `Fsm`, not `FSM`.

### Ledger DTOs

- `FsmEventV1` → `FsmEventAsLedgerEntry`
- `FsmStateV1` → `FsmStateAsLedgerEntry`
- `DomainActionV1` → `DomainActionAsLedgerEntry`
- `LedgerPayloadV1` → `LedgerEntry`
- `WheelRpmV1` → `WheelRpmAsLedgerEntry`
- `PowertrainContextV1` → `PowertrainContextAsLedgerEntry`
- `HealthContextV1` → `HealthContextAsLedgerEntry`
- `VisibilityContextV1` → `VisibilityContextAsLedgerEntry`
- `WeatherContextV1` → `WeatherContextAsLedgerEntry`
- `WiperStateV1` → `WiperStateAsLedgerEntry`
- `WiperContextV1` → `WiperContextAsLedgerEntry`
- `ObservedBoolV1` → `ObservedBoolAsLedgerEntry`
- `SccmContextV1` → `SccmContextAsLedgerEntry`
- `BcmStateV1` → `BcmStateAsLedgerEntry`
- `BcmContextV1` → `BcmContextAsLedgerEntry`
- `FlcmContextV1` → `FlcmContextAsLedgerEntry`
- `HeadlampStateV1` → `HeadlampStateAsLedgerEntry`
- `HeadlampContextV1` → `HeadlampContextAsLedgerEntry`
- `VehicleContextV1` → `VehicleContextAsLedgerEntry`
- `FrontHeadlampSwitchDirectionV1` → `FrontHeadlampSwitchDirectionAsLedgerEntry`
- `FrontHeadlampIncompleteCauseV1` → `FrontHeadlampIncompleteCauseAsLedgerEntry`
- `OperationalV1` → `OperationalAsLedgerEntry`

### Diagnostic DTOs

- `DiagnosticLevelV1` → `DiagnosticLevelAsDiagnosticEntry`
- `DiagnosticKindV1` → `DiagnosticKindAsDiagnosticEntry`
- `DiagnosticPayloadV1` → `DiagnosticEntry`

### Manifest DTOs

- `ManifestV1` → `ManifestEntry`
- `VehicleV1` → `VehicleAsManifestEntry`
- `StreamsV1` → `StreamsAsManifestEntry`

### Shared persisted values

- `UnixTimestampV1` → `UnixTimestampAsPersistedValue`
- `StreamEnvelopeV1<T>` → `StreamEnvelope<T>`

`RunId`, `RunMetadata`, and `ScenarioMetadata` already describe their roles without a false version suffix and retain their names. Remove the legacy `Timestamp` alias: repository search shows no internal consumer beyond its public re-export, and retaining it would leave two names for the same persisted timestamp.

## Version contract

- Set `CURRENT_SCHEMA_VERSION` to `9`.
- Keep `MIN_SUPPORTED_SCHEMA_VERSION` at `1`.
- `RunWriter` writes version 9 to the manifest and every stream envelope.
- `RunReader` accepts versions 1–9.
- A stream row must still match its manifest's version.
- Version 9 is required for `assembly_zone_ready`.
- Update module, reader, writer, and public API comments to state:
  - current writer schema: 9;
  - supported reader range: 1–9;
  - all DTOs in `schema::entries` represent the current writer shape and the additive historical read shape.

The reader is upgraded as part of this change. Backward compatibility does not preserve old Rust names or old reader code; it preserves the ability of the new reader to load existing captured runs.

## Truthful assembly-ready observation

Add an assembly identifier to the observation boundary:

- `PublishedAssemblyId` in the live published projection.
- `AssemblyIdAsLedgerEntry` in the persisted schema.
- `PublishedFsmEvent::AssemblyZoneReady(PublishedAssemblyId)`.
- `FsmEventAsLedgerEntry::AssemblyZoneReady { assembly: AssemblyIdAsLedgerEntry }`.

Projection must preserve `Sccm`, `Bcm`, `Flcm`, `Headlamp`, and `Wiper` without wildcard or placeholder conversion. JSON uses the existing snake-case tagged-event convention:

```json
{"type":"assembly_zone_ready","assembly":"flcm"}
```

This is additive schema-9 vocabulary. Schemas 1–8 remain readable; newly written records always use schema 9.

## Test design

Implementation follows test-driven development.

### Projection unit tests

First add a failing test that projects every `AssemblyId` through:

`FsmEvent::AssemblyZoneReady` → `PublishedFsmEvent::AssemblyZoneReady` → `FsmEventAsLedgerEntry::AssemblyZoneReady`.

The test must fail against the current `TimerTick` mapping before production code changes. Assertions compare concrete variants and assembly identifiers.

### Version-uniformity tests

Write one captured run and assert:

- manifest version is 9;
- every diagnostic envelope version is 9;
- every ledger envelope version is 9;
- rows whose version differs from the manifest are rejected.

### Serialization tests

- Assert the exact schema-9 JSON for every assembly ID.
- Round-trip the new event through Serde and through the live conversion functions.
- Keep unknown event tags rejected.
- Verify ordinary pre-existing events retain their prior JSON representation.

### Compatibility tests

- Load existing golden fixtures for versions 1–8 with the upgraded `RunReader`.
- Add a schema-9 golden fixture containing `assembly_zone_ready`.
- Change the unsupported-future-version test from version 9 to version 10.
- Update stale comments and assertions that name version 8 as current.

### Rename verification

`cargo check --workspace --all-targets` provides compile-time coverage for all renamed Rust APIs. Schema tests provide wire-format coverage because compile success cannot detect accidental JSON changes.

## Migration record for the original Twin project

The implementation must leave these artifacts:

1. This approved design specification.
2. A file-by-file implementation plan with exact rename and test order.
3. A dedicated implementation commit or small ordered commits whose messages mention schema 9.
4. A schema-9 golden run containing an assembly-ready ledger row.
5. Compatibility tests covering versions 1–9.
6. Release/migration notes listing:
   - old and new Rust module paths;
   - the complete type rename map;
   - the schema-version bump reason;
   - the unchanged JSON contract;
   - the one new JSON event;
   - commands used for verification.
7. No temporary compatibility aliases for the old `*V1` Rust names unless a downstream consumer is discovered and documented. Compile failures should identify migration sites directly.

These records are sufficient to replay the migration in the original Twin repository without relying on chat history.

## Acceptance criteria

1. `common::fsm::FsmEvent` remains the sole authoritative runtime FSM event type.
2. No current observation DTO or schema module is named `V1` or `v1`.
3. Persisted DTO names identify ledger, diagnostic, manifest, or shared-value roles.
4. All newly written observation artifacts declare schema version 9.
5. The upgraded reader loads golden schema versions 1–9.
6. `AssemblyZoneReady` and its assembly ID survive runtime-to-ledger projection without substitution.
7. A unit test fails on the old `AssemblyZoneReady → TimerTick` behavior and passes after the fix.
8. Existing JSON remains unchanged except for the additive schema-9 event.
9. Workspace checks and tests pass.
10. The migration record described above is committed with the implementation.
