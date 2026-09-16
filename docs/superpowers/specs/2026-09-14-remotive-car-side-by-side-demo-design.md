# Phase III — RemotiveCar side-by-side observation demo

**Date:** 2026-09-14  
**Branch baseline:** `remotive-integration` at Phase II head (schema v5 + SCCM/BCM observation)  
**Status:** Design approved in brainstorming; awaiting implementation plan

## Goal

Run unchanged RemotiveCar Hello World (including the 3D “cute car”) beside the
Twin TUI console so a Jupyter hazard press is visible on both surfaces at once.
The Twin continues to observe only the three Phase II signals; no new ECU
observation vocabulary is added in this phase.

## Architecture

```text
Jupyter (Hello World, this phase only)
  → SCCM HazardLightButton
  → Remotive Python BCM
  → Left/Right TurnLightControl
  ├→ 3D car (:3000)
  └→ remotive_bridge
       [subscribe ONLY the three Phase II identities]
       → Gateway → Twin SCCM/BCM children
       → TUI (hazard / left / right + speed)
```

### Ownership

| Component | Owns |
|-----------|------|
| Remotive Hello World (unchanged) | Full ECU topology, BCM policy, 3D car, Jupyter stimulus |
| `remotive_bridge` | Exact three-signal subscribe; Twin lifecycle `PowerOn` / RPM (`--rpm-clamp`, default 1000) / `PowerOff` |
| Gateway / Twin | Observe hazard + left + right; ledger; live TUI feed |
| Twin TUI | Present attended observations + current speed |

- No Twin→Remotive publisher in Phase III.
- No RemotiveLabs source edits.
- No new formal runbook document; the implementation plan lists shell commands
  in the correct order across separate terminals.

## Twin signal set (unchanged from Phase II)

Bridge subscription remains exactly:

1. `SCCM-DriverCan0:HazardLightButton.HazardLightButton`
2. `BCM-BodyCan0:TurnLightControl.LeftTurnLightRequest`
3. `BCM-BodyCan0:TurnLightControl.RightTurnLightRequest`

Internal carriers stay `0x105` / `0x106` / `0x107`. The bridge must not
subscribe to other Hello World signals.

## TUI presentation

**Show (attended):**

- Hazard / Left request / Right request as `UNKNOWN` / `OFF` / `ON`
- SCCM/BCM engineer lines for those values
- Current speed derived from bridge-generated RPM (session context, not an ECU
  observation)
- Lifecycle standby / PowerOn–PowerOff as needed for session clarity
- Ledger emphasis on the three observed events (plus lifecycle)

**Hide / do not present as attended Twin observations:**

- Headlamp, wiper, weather, ambient lux, rain
- Any other Hello World ECU outputs (beams, brake, stalks, ABS, etc.)

Twin may still ingest RPM for lifecycle/FSM arming; Phase III only requires that
unattended domains are not shown as primary demo signals beside the cute car.

## Stimulus

- **This phase:** Jupyter-driven hazard (and related Hello World controls as
  needed to exercise hazard → BCM turn requests).
- Automated Hello World tester/behave may be used as optional regression but are
  not required to define Phase III success if Jupyter + visual proof is the
  accepted demo path.
- **Later (deferred):** continuous series of inputs may move from bridge/Jupyter
  into Remotive; not in Phase III scope.

## Run shape (commands in separate shells)

Ordered sequence only (exact commands belong in the implementation plan):

1. Prepare `vcan0`.
2. Build and start unchanged Hello World with profiles needed for 3D car and
   Jupyter (CAN-over-UDP / VLAN settings as required on this host).
3. Open 3D car UI.
4. Start Gateway (UDS bind-first, no `--print-transitions-only` on the TUI
   path), then TUI, then bridge to the Hello World broker URL.
5. Confirm three-signal readiness before Twin `PowerOn`.
6. Use Jupyter to press hazard ON; confirm cute car and TUI together.
7. SIGINT bridge for clean `RPM=0` / `PowerOff`; tear down only this compose
   project; for a full session end, stop `remotivebusd` only after compose is
   confirmed down (Twin → compose → RemotiveBus).

Do not delete unrelated Docker networks (e.g. leftover projects).

## Acceptance

1. Remotive Hello World source tree remains unmodified (generated build outputs
   may change).
2. Bridge readiness line lists exactly the three Phase II signal identities
   before `PowerOn`.
3. Jupyter hazard ON → 3D car shows hazard/turn effect; TUI shows hazard ON and
   left/right ON.
4. No Twin `SetTurnLights` (or equivalent actuation back into Remotive).
5. TUI does not present headlamp/wiper/weather/lux/rain as attended observations;
   speed from bridge RPM remains visible.
6. Controlled shutdown leaves no Phase III–owned Twin/compose leftovers; for a
   full session end, RemotiveBus is inactive after compose is down.

## Explicitly deferred

- Generating visibility from an ECU (e.g. tunnel) instead of / in addition to
  bridge environment generation.
- Twin actuating hazard lights back into Remotive.
- Expanding Twin observation to beams, brake, turn stalk, or other Hello World
  ECUs.
- Moving continuous stimulus generation from bridge/Jupyter into Remotive.
- A dedicated Phase III runbook markdown file.

## Relationship to Phase II

Phase II proved authoritative ECU observation on `getting_started`. Phase III
reuses that Twin/bridge contract against Hello World’s richer Remotive surface
for a side-by-side visual demo. Product behavior of the three-signal path does
not change except for TUI cleanup of unattended domains.
