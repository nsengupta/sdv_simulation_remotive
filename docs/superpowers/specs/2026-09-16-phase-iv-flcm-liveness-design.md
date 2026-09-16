# Phase IV — FLCM liveness fault + Twin Warning / low-beam observation

**Date:** 2026-09-16  
**Branch baseline (Twin):** `remotive-integration` (Phase III side-by-side + hazard-mode latch)  
**Branch policy (Remotive):** all Hello World / topology edits on a **new dedicated branch** in the local Remotive tree so a RemotiveLabs PR (if offered) is a clean tip; Twin repo changes stay on the Twin integration branch  
**Status:** Design approved in brainstorming; awaiting implementation plan

## Goal

Demonstrate that the Twin can detect when Remotive’s **Front Light Control Module (FLCM)** stops sending continuous BodyCan updates, raise a **TUI Warning**, and still present **low-beam L/R** state when frames are flowing (and last-known values while silent, if held).

Remotive remains the source of lamp traffic and of the **fault inject**. The Twin does **not** actuate Remotive; it **observes** FLCM liveness and low-beam signals.

## Problem

Phase II/III Twin observation covers SCCM hazard + BCM turn requests only. Hello World’s FLCM is a broker restbus ECU (`FLCM: {}`) that cyclically participates on BodyCan. If that cyclic traffic stops, the **3D cute car** typically **holds last lamp values** (no fault chrome). Without a Twin liveness path, the attended console also has no Warning — so an ECU “went quiet” fault is invisible on the Twin side.

## Architecture

```text
Jupyter / fault hook (Remotive local branch)
  → pause FLCM cyclic BodyCan TX (Approach A)
  → FLCM-BodyCan0 traffic stops (or resumes)

remotive_bridge
  → subscribe: Phase II three signals (unchanged)
  → PLUS FLCM liveness feed + LowBeam L/R (exact IDs locked in plan spike)
  → Gateway → Twin

Twin
  → liveness timer on last FLCM traffic → diagnostic / Warning when stalled
  → low-beam L/R into observed context for TUI
  → TUI: Warning on silence; attended low-beam when known
```

### Ownership

| Component | Owns |
|-----------|------|
| Remotive Hello World (**new local branch**) | Fault-injectable pause/resume of FLCM cyclic TX; minimal Python |
| `remotive_bridge` | Expanded subscribe: FLCM liveness + `LowBeamLightControl` L/R; still no Twin→Remotive publisher |
| Gateway / Twin | Silence detection → Warning; beam values in published context |
| Twin TUI | Warning surface; attended low-beam L/R |
| Phase IV docs (Twin repo) | Spec, plan, and updated Remotive **run steps** for fault inject + Twin attach |

### Branching / PR posture

- **Remotive topology / Hello World:** create and keep work on a **new branch** (name chosen at plan start, e.g. `demo/flcm-liveness-fault`). Do not mix unrelated Remotive edits on that branch. If RemotiveLabs agrees, open a PR from that branch; otherwise the branch remains local / private fork.
- **Twin (`sdv_simulation_remotive`):** implement Phase IV on the Twin integration branch; document the Remotive branch name and run commands in the Phase IV plan / run steps.
- License / private GitHub copy: operator’s choice; this design does not require publishing Remotive changes.

## Approach (locked)

**Approach A — fault-injectable FLCM silence**, with lock-in **C** (heartbeat for Warning **and** beam values for TUI lamp state).

Remotive implementation preference (minimal Python):

1. **First spike:** pause/stop FLCM cyclic TX via existing Jupyter / `BrokerClient` / restbus APIs against `FLCM-BodyCan0` (notebook cell + docs only). Preferred because FLCM today has **no** behavioral Python model.
2. **Fallback only if spike fails:** smallest local FLCM behavioral/restbus stub that can pause on a flag or Jupyter-driven signal — not a full lamp policy rewrite; not a BCM beams rewrite.

Twin implementation:

- Treat **recent FLCM namespace/cyclic traffic** as alive (heartbeat).
- After silence longer than threshold `T` (concrete default in plan), emit/raise a **Warning** on TUI; clear when traffic resumes.
- Observe **low-beam L/R** for attended TUI state (last known may remain displayed during silence).
- Do **not** reuse Twin→actuator `FrontHeadlampActuationIncomplete` / ACK timeout for this story; that path is actuation ACK, not Remotive observation silence.

Cute car:

- Leave `3d_car_mapping.yaml` unchanged unless a spike proves a mapping bug.
- Expected during silence: **last-value hold** on low beam / related indicators — intentional contrast with Twin Warning.

Rejected for Phase IV:

- Container kill as the primary demo (too blunt; optional emergency only)
- Random unsupervised stalls without a controllable inject
- Inferring FLCM health only from BCM request lines without FLCM-side silence
- Twin→Remotive actuation

## Twin / bridge contracts (intent)

- Phase II three-signal hazard/turn path **keeps working**.
- Bridge readiness / subscription list **grows** by the FLCM liveness + low-beam identities chosen in the plan spike; document the exact strings in the plan and run steps.
- Schema / published context: add only what TUI needs for Warning + low-beam (version bump if required by existing observation rules).
- Warning is an attended operator signal (Notice/diagnostic path as used by TUI today), not cute-car chrome.

## Run shape (docs must update)

Phase IV plan must list ordered shells, including Remotive on the **dedicated branch**:

1. Check out Remotive demo branch; build/start Hello World (Jupyter + 3D car profiles as in Phase III).
2. Twin: `vcan0`, Gateway (UDS), TUI, bridge (expanded subscribe; `--rpm-clamp` as needed).
3. Confirm Phase III hazard path still works.
4. Inject FLCM silence (Jupyter cell or documented hook); confirm TUI Warning within `T`.
5. Confirm low-beam attended when live; cute car may look frozen during silence.
6. Resume FLCM; Warning clears.
7. Controlled shutdown (Twin → compose → RemotiveBus as already documented).

## Explicitly deferred

- **Asymmetric hazard:** hazard mode ON while only one side blinks (L/R disagree). Cute car looks one-sided wrong; Twin would show whole-car inconsistency. Not Phase IV.
- Twin→Remotive actuation.
- Environment / visibility from an ECU (e.g. tunnel) instead of bridge generation.
- Blinking cosmetic `Hazard:` paint (separate from latch).

## Acceptance

1. Remotive edits live only on the dedicated Remotive branch (clean tip for a possible RemotiveLabs PR).
2. Controllable FLCM silence inject; Twin TUI shows **Warning** within threshold `T`; Warning clears on resume.
3. Low-beam L/R attended on TUI when FLCM/beam traffic is live.
4. Phase III hazard + latch path still works with the expanded bridge subscribe.
5. Phase IV plan/run steps document exact Remotive branch, inject command/cell, and Twin attach order.
6. No Twin `SetTurnLights` (or equivalent) into Remotive for this phase.

## Relationship to Phase III

Phase III proved side-by-side observation (three signals + cute car). Phase IV **widens** observation and adds **ECU liveness** as a major Twin integration value demo, with intentional local Remotive fault inject on a PR-friendly branch.
