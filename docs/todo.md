# TODO

## Current Status

- Initial Rust/WASM scaffold in place.
- Notes and early art assets imported.
- Documentation baseline created.

## Next Steps

### 1. Define Core Rules
- [ ] Define action types and legality rules.
- [ ] Define push/pull/charged push effects and damage rules.
- [ ] Define flag pickup, drop, return, and win conditions.

### 2. Build State Model
- [ ] Define match state data model.
- [ ] Define turn order and action point system.
- [ ] Define hazard tiles and interaction hooks.

### 3. Tests First
- [ ] Unit tests for action legality and resolution.
- [ ] Determinism tests for repeated simulations.
- [ ] Victory condition tests.

### 4. Implement Core Simulation
- [ ] Implement resolver that applies actions to state.
- [ ] Implement event log for UI and replay.
- [ ] Add seeded RNG for deterministic NPC spawns.

### 5. Human Faction (NPC)
- [ ] Define archetype behaviors.
- [ ] Define wave escalation rules.
- [ ] Implement wave manager.

### 6. UI / Playtest Loop
- [ ] Minimal board view and unit selection.
- [ ] Basic action UI and turn timer.
- [ ] Local hotseat playtest flow.

## Questions to Resolve

- Lockstep vs authoritative netcode for online play.
- Final map dimensions and lane layout rules.
- Hazard list and how they interact with pushes.

