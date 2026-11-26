# Shell

## Invariants

- The shell keeps five built-ins (`ls clear rose sleep random`) and resolves every other prompt name through VFS: a name starting with `/` is used as-is, any other name is stat-walked through the `system::boot_disk` search path (`/bin → /tests → /games → /bench → /stress`), first hit wins; the volume root and `srv/` are off the path.
- Built-ins run inside the shell task and cannot detach; `&` on one is announced and run in the foreground.
- Finished background jobs are reaped only at prompt boundaries; there is no tty or job control.

## Detail

The shell is a user ELF executed by init from the mounted FAT root, falling back to the ROM copy when the disk is missing. It keeps five built-ins (`ls clear rose sleep random`) and resolves every other prompt name through VFS — leading-`/` names as-is, bare names through the `system::boot_disk` search path (`/bin → /tests → /games → /bench → /stress`; services stay off it so a stray `pong` cannot race a running daemon for its bind-name), a full miss printing `Unknown command: <name>` — so disk files execute by bare name with arguments (`rose> snake fast`). A trailing `&` token backgrounds a launched program: the shell loads the ELF without waiting on its completion future, announces `<name> started`, and reaps finished jobs at every prompt boundary — a non-consuming `IsFutureCompleted` scan, then a `WaitFuture` that consumes the future (returning its registry slot, the one leak guard a detached launch needs) and prints `finished` / `exited with code N` / `panicked`, so notices surface at the next prompt, never asynchronously. Built-ins execute inside the shell task and cannot detach; `&` on one is announced and run in the foreground. Background output interleaves with the prompt: there is no tty or job control.
