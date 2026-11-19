---
priority: low
date: 2026-10-20
trigger: none
---

# Revert the underscore-free ROM app names

The ROM app names were renamed underscore-free on 2026-10-20 to mask a keyboard gap. Shifted characters are now typeable (the `ps2d` decoder keeps shift state), so the renames can be reverted as a separate task. CapsLock stays out of scope.
