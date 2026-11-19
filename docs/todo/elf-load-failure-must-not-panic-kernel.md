---
priority: high
date: 2026-10-19
trigger: any untrusted-bytes source for executables (RAM files are already writable)
---

# ELF load failure must not panic the kernel

`elf_task_wrapper` (`kernel/src/task.rs`) calls `load_elf(...).unwrap()` when the task first runs. That was safe while every schedulable ELF was baked in at build time; now the shell resolves unknown names as files, so `rose> notes.txt` or a corrupted blob turns a typo into a kernel panic.

Match on the `load_elf` result: on `Err`, print a diagnostic (task name and `ElfError` debug) through kernel output and terminate the task without entering the entry point. Prove with a kernel build plus a QEMU run scheduling a non-ELF blob; host-testable decision logic gets unit tests.
