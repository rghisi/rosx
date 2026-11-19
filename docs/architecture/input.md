# Input

## Invariants

- Input policy lives in the user-space `inputd` daemon and device semantics in driver daemons (`ps2d`); the kernel delivers opaque event frames only.

## Design

Input architecture — policy lives in the user-space `INPUTD` daemon (`inputd`), device semantics in driver daemons (`ps2d`); the kernel delivers opaque event frames only.
