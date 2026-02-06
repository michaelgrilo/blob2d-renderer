# Project Architecture

## Development Flow (DTI)

1. Define interfaces and rules first (types and contracts only).
2. Write tests that codify the expected behavior.
3. Implement minimal code to make tests pass.
4. Optimize or refactor after correctness is proven.

For details, see [Methodology](./methodology.md).

## Layers Overview

- Rules Layer: game rules, action legality, combat math.
- Simulation Layer: turn resolver, match state transitions, deterministic RNG.
- AI Layer: NPC (Human faction) behaviors and wave escalation.
- Presentation Layer: UI state, rendering bindings, input handling.
- Asset Layer: sprite and data loading, caching, and validation.

## Core Components

1. Match State
   - Turn order, action points, positions, hazards, and flag state.
2. Action System
   - Push, pull, charged push, and environment interactions.
3. Resolver
   - Applies actions to state and produces events.
4. Wave Manager
   - Spawns NPC waves and boss escalations.
5. Victory Manager
   - Evaluates win conditions after each action/turn.

## Module Layout (Draft)

```
src/
  core/          # Rules and data types
  sim/           # Turn resolver and deterministic updates
  ai/            # Human faction behaviors
  ui/            # UI bindings and view models
  assets/        # Asset loading and metadata
  tests/         # Unit and integration tests
```

## Testing Strategy

- Unit tests: rules, action legality, and resolver outcomes.
- Integration tests: wave escalations and win conditions across turns.
- Determinism tests: identical inputs produce identical outcomes.

## Determinism and State

- No hidden randomness; all RNG must be seeded and recorded in match state.
- All state transitions happen through the resolver.
- UI reads state but does not mutate it directly.

## Performance Notes

- Favor data-oriented structures for fast simulation steps.
- Keep UI updates incremental and derived from state diffs.

