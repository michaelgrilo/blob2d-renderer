# Define-Test-Implement (DTI)

DTI is a structured workflow to move from idea to correct code:

1. Define - write exactly what needs to be built.
2. Test - write tests from the definition checklist.
3. Implement - write the minimum code to pass tests.

## Phase 1 - Define

- Describe the behavior in plain language.
- List inputs, outputs, invariants, and edge cases.
- Turn the definition into a checklist.

## Phase 2 - Test

- Convert each checklist item into a test.
- Cover normal flows and edge cases.
- Use tests to keep AI and human changes aligned with intent.

## Phase 3 - Implement

- Make the smallest change that passes the next failing test.
- Only refactor after correctness is proven.

## Benefits

- Lower cognitive load: one phase, one job.
- Clear expectations for AI assistance.
- Deterministic, testable rules for game systems.
- Safer iteration as rules expand.

