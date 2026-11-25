---
status: decided
date: 2026-11-25
revisit-when: a service-manager daemon is designed, or a consumer needs a service that the boot plan cannot provision
---

## Question
Who brings up a daemon when a consumer CLI (e.g. `ip`) finds its binding absent:
the consuming app on demand, the boot plan, or a future service manager?

## Chosen
Never the consuming app. Services are provisioned ahead of use — non-optional
ones by the boot plan (`pcid`, `ne2kd` are plan services from `/srv/`), `ipd`
manually (`/srv/ipd &`, backgroundable since the shell gained `&` job control).
A CLI whose binding is missing reports `no service` and exits. The 2026-11-17
auto-launch rationale in `docs/architecture/network-and-ip.md` (CLIs loading
`pcid`/`ne2kd`/`ipd` via `load_with_args` because "the shell itself blocks on
launched programs, so daemons never come up from a prompt") is superseded: its
premise predates `&`, and the mechanism coupled three app CLIs to VFS and to
the `system::boot_disk` media layout while duplicating the shell launcher's
resolve-and-load path a second time. Removed in `268e341`.

## Alternatives
Keep the in-app auto-launch: keeps per-consumer copies of placement policy and
the daemon bootstrap, and every future CLI would repeat it. Add `service
/srv/ipd IP` to the boot plan: zero probe cost (`ipd` binds `IP` before
attaching the link), but `ipd` configuration is manual anyway, so a plan-started
daemon idles unconfigured on boots that never touch the network; rejected for
now, cheap to add later. A kernel-side name resolver: inverted authority — the
kernel never reads the VFS and resolves no program names.

## Consequences
`ip`, `ne2k` and `arp` are thin protocol clients: their crate arrows are only
`usrlib`, `net-protocol` and (for `ip`) `ip-protocol`. The network suites bring
`ipd` up explicitly through `tests_integration::shared::start_ipd`. On-demand
start and restart of daemons are the unclaimed domain of a future service-manager
daemon, which would own the VFS and `system::boot_disk` knowledge in one place
and expose an `ensure`-style IPC to consumers.
