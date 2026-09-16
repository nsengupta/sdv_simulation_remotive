# Phase IV — FLCM lamp status feedback + Twin Warning (cute car stays blind)

**Date:** 2026-09-16  
**Branch baseline (Twin):** `remotive-integration` (Phase III side-by-side + hazard-mode latch)  
**Branch policy (Remotive):** all Hello World / topology edits on a **new dedicated branch** in the local Remotive tree so a RemotiveLabs PR (if offered) is a clean tip; Twin repo changes stay on the Twin integration branch  
**Status:** Design approved in brainstorming (Approach A); awaiting implementation plan  
**Revision:** Corrected after DBC scan — FLCM had no TX today; Phase IV **adds** FLCM status/liveness TX and Twin observation. Cute car remains on BCM *requests*.

## Goal

Show Twin integration value: **BCM still commands front low beams** and the **3D cute car still looks ON**, while **FLCM** reports lamp **Status** (OK / Fail) and cyclic **liveness**. When FLCM fails or goes silent, **only the Twin TUI warns** — Remotive “brain” (BCM) and cute car stay unaware.

Remotive owns fault inject on a PR-friendly branch. Twin **observes** only (no Twin→Remotive actuation).

## Problem

### What Hello World does today

- BodyCan light frames (`LowBeamLightControl`, DRL, turn, …) are **sent by BCM**, received by FLCM / RLCM / DIM (`platform/databases/body_can.dbc`).
- `FLCM: {}` is a broker-attached ECU with **no behavioral model** and **no FLCM-authored TX frames**.
- Cute car maps **BCM request** signals (e.g. `LowBeamLightControl.*Request`) — not lamp ECU feedback.
- No Hello World brain (BCM / GWM / …) times out another ECU’s “I-am-working” signal. Closest pedagogy is RLCM↔RL LIN `counter` / `counter_times_2`, and **RLCM never checks** the slave reply — same open-loop class of gap.

### Why that matters

Command path can look healthy on the webpage while a front lamp ECU is dead or lying. Twin observation of **FLCM truth** is the value-add Phase IV must create (it does not exist yet).

## Architecture

```text
Jupyter light stalk → SCCM → BCM
  → BodyCan LowBeamLightControl *Request*
       ├→ 3D cute car (UNCHANGED mapping) → beams look ON
       └→ FLCM (new stub on Remotive branch)
            → BodyCan LowBeamLightStatus L/R (+ cyclic alive)
                 ├→ BCM / GWM: do NOT consume in Phase IV (blind)
                 └→ remotive_bridge → Gateway → Twin → TUI Warning

Fault inject (Jupyter / hook):
  FLCM Status=Fail and/or mute FLCM cyclic TX
  BCM requests keep flowing → cute car still ON
```

### Ownership

| Component | Owns |
|-----------|------|
| Remotive Hello World (**new local branch**) | DBC: FLCM-authored status/liveness frames; tiny FLCM stub/restbus; controllable fault inject (minimal Python); leave cute-car mapping and BCM beams policy alone |
| `remotive_bridge` | Keep Phase II three signals; **add** FLCM Status L/R + liveness subscribe (exact IDs in plan spike) |
| Gateway / Twin | Map FLCM Status + silence → Warning / context for TUI |
| Twin TUI | Warning on Fail or silence; attended low-beam status (OK / Fail / Unknown) |
| Phase IV docs (Twin repo) | Spec, plan, Remotive branch name, inject + Twin run steps |

### Branching / PR posture

- **Remotive:** dedicated branch (e.g. `demo/flcm-lamp-status-feedback`). Clean tip for optional RemotiveLabs PR (“add front lamp status ECU + demo fault inject”).
- **Twin:** Phase IV on Twin integration branch; docs name the Remotive branch and commands.
- Private fork / license: operator’s choice; publishing Remotive changes is optional.

## Approach (locked) — A

**FLCM Status + silence; BCM and cute car ignore FLCM feedback.**

### Remotive (minimal Python)

1. **`body_can.dbc`:** add cyclic frame(s) with **sender = FLCM** (e.g. left/right low-beam **Status**, optional rolling counter). Receivers may list BCM/DIM for realism; **BCM Python must not consume them in Phase IV**.
2. **FLCM stub:** replace empty `FLCM: {}` with the smallest restbus/behavioral stub that:
   - normally publishes Status=OK (or mirrors “commanded on” as healthy) on a cycle;
   - on fault inject: Status=Fail and/or stops cyclic TX;
   - on resume: restores OK + TX.
3. **Inject surface:** Jupyter cell or one-shot hook (prefer notebook-only control once stub exists).
4. **Do not** rewrite BCM `BeamsStateMachine` or change `3d_car_mapping.yaml` for Phase IV.

### Twin

- Subscribe to FLCM Status L/R + treat recent FLCM status/liveness TX as alive.
- **Warning** when Status=Fail **or** silence longer than threshold `T` (default in plan).
- Clear Warning when Status=OK and traffic resumes.
- Attended TUI low-beam status from FLCM (not from BCM request lines alone).
- Do **not** use Twin→actuator `FrontHeadlampActuationIncomplete` ACK path for this story.

### Cute car (intentional blindness)

- Keep mapping to BCM `LowBeamLightControl.*Request`.
- During FLCM Fail/silence with BCM still commanding ON: **webpage beams stay ON** — contrast with Twin Warning is the demo.

## Twin / bridge contracts (intent)

- Phase III hazard + latch path still works.
- Bridge subscription list grows by FLCM identities only (document exact strings in plan/run steps).
- Schema / published context: only what TUI needs for Warning + FLCM low-beam status (version bump if observation rules require).
- Warning uses existing attended Notice/diagnostic path — not cute-car chrome.

## Run shape (docs must update)

1. Check out Remotive **demo branch**; build/start Hello World (Jupyter + 3D car).
2. Twin: `vcan0`, Gateway (UDS), TUI, bridge (expanded subscribe; `--rpm-clamp` as needed).
3. Set low beams ON via Jupyter; confirm cute car ON + TUI Status OK.
4. Inject FLCM Fail and/or silence; confirm **TUI Warning** within `T` while **cute car still ON**.
5. Resume FLCM; Warning clears; Status OK.
6. Confirm Phase III hazard path still works.
7. Controlled shutdown (Twin → compose → RemotiveBus).

## Explicitly deferred

- Teaching **BCM** to react to FLCM Fail (would close the Remotive loop and weaken Twin-only contrast).
- Remapping cute car to FLCM Status (same).
- **RLCM↔RL** slave-reply validation (rear LIN; good future PR, not this webpage demo).
- **SCCM brake E2E** timeout (automotive-authentic, wrong domain for front lamps).
- **Asymmetric hazard** (mode ON, one side dead).
- Twin→Remotive actuation; environment-from-ECU; blinking Hazard paint.

## Acceptance

1. Remotive edits only on the dedicated Remotive branch.
2. With low beams commanded ON: cute car shows ON; TUI shows FLCM Status OK when healthy.
3. Fault inject → TUI **Warning** (Fail and/or silence within `T`) while cute car **remains ON**.
4. Resume → Warning clears.
5. Phase III hazard + latch still works with expanded bridge subscribe.
6. Plan/run steps document Remotive branch, inject cell/command, Twin attach order.
7. No Twin actuation into Remotive for this phase.

## Relationship to Phase III

Phase III proved side-by-side observation of hazard/turns. Phase IV adds the **missing closed-loop signal from FLCM**, leaves Remotive visuals on the open-loop BCM request path, and makes the **Twin** the observer that traps lamp-ECU malfunction — the integration value Hello World does not provide alone.
