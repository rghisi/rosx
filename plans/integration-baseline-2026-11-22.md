# Integration Test Baseline — 2026-11-22 (Step 0 of the speed-up plan)

> **Date:** 2026-11-22
> **Commit:** `7c4a5f1` (branch `integration-speedup`, base `origin/main`)
> **Purpose:** reference numbers for `plans/integration-speedup-plan.md` —
> per-test time, boot count, wall total and flakes, before any change.
> **Companions:** `plans/integration-baseline-2026-11-07.md` (earlier snapshot,
> same table style) and `plans/integration-flakiness-assessment-2026-11-20.md`
> (flake classes M1–M9 referenced below).

## Method

- Serial protocol per arch, exactly the plan's Step 0 command:
  `ROSX_KERNEL_ELF=<release kernel> [ROSX_ARCH=x86_32] cargo test --no-fail-fast -p tests-integration -- --test-threads=1 -Zunstable-options --report-time`
- Release kernels from `cargo xtask build` / `cargo xtask build x86_32`
  (`target/rosx/release/rosx`, `target/rosx-i486/release/rosx-x86_32`).
- Boot-phase split from `measure_boot` (`-- --ignored --nocapture
  boot_to_prompt_distribution`, 5 samples per arch).
- **Quiet-host gate (Nov-21 erratum honoured):** every measurement step was
  chained behind `pgrep 'qemu-system|cargo test'` returning empty. A foreign
  OpenCode session (worktree `tidy-forest`) ran its own `cargo test
  -p tests-integration` from 07:32; all four measurements started only after
  it finished (25 min gated wait, then 0 waits for the remaining three steps).
  Measurement windows: x86_64 suite 08:03–08:18, x86_64 boot 08:18–08:19,
  x86_32 suite 08:19–08:31, x86_32 boot 08:32.

## Measurement host

Linux 7.0.0-38-generic, 4 cores / ~7.8 GiB, QEMU 10.2.1, rustc
1.101.0-nightly (0abfedbc7 2026-10-02). `/dev/kvm` exists but the runner does
not enable it — all timings are pure TCG (the 2.8 s SeaBIOS+stage2 leg at
TCG speed confirms).

## Boot-to-prompt distribution (release kernels, 5 samples)

| Stamp (from spawn) | x86_64 release | x86_32 release |
|---|---|---|
| session spawn (image creation + exec) | 0.025 s | 0.009 s |
| `[KERNEL] Initializing` | 2.84 s | 0.42 s |
| `[KERNEL] Starting` | 2.86 s | 0.44 s |
| `rose>` prompt | **14.69 s** | **11.64 s** |

**Findings:**

1. The kernel→prompt leg is **~11.8 s (x86_64) / ~11.2 s (x86_32)** — the
   `init` boot plan. It has become the dominant boot cost on *both* arches:
   since the 2026-11-07 baseline the plan added services and the x86_64
   prompt leg grew 3.9 s → 14.7 s.
2. The `ne2kd` probe tax (M8) is ~10 s of that leg on every NIC-less boot —
   which is nearly every boot. The remaining ~1.8 s (x86_64) / ~1.2 s
   (x86_32) is real service bring-up (vfsd, inputd, ps2d, blockd, atad, …).
3. The x86_32 boot medium is the unified-disk chain now (0.42 s to banner):
   the 11-07 `-kernel`-multiboot asymmetry (0.8 s boots) is gone. Both arches
   pay the same init tax, so the Step 1 probe fix buys both equally.
4. Samples were tight on both arches (prompt spread 0.06 s x86_64, 0.10 s
   x86_32) — the gated quiet host worked; timing gates must still stay
   generous.

## Suite baselines

### x86_64 — wall 15 m 07 s (907 s), test sum 895.8 s

59 tests: 56 spawn a VM (`disk_boot` ×3 skip on this arch), 3 of them boot a
second VM in-test → **≈59 VM boots**. 1 failed test (flake, see below). The
floor for a simple prompt-driven test is ≈15.0 s.

| Test | s | Test | s |
|---|---:|---|---:|
| `vfs_hdd_flow::register_format_and_carve_directories_on_the_hard_disk_volume` | 40.00 | `shell_background::a_panicking_background_job_is_reported_at_a_later_prompt` | 15.19 |
| `partitioned_hdd_flow::slices_are_first_class_devices_on_a_partitioned_disk` | 35.60 | `shell_command_arguments::shell_forwards_command_arguments_to_elf_app` | 15.19 |
| `fat_partition_flow::fat_partition_mounts_by_autodetection_and_survives_reboot` | 35.38 | `ata_write::ata_write_roundtrips_on_the_scratch_slave` | 15.18 |
| `ctrl_c::ctrl_c_kills_the_running_foreground_app` ⚠FAILED | 33.74 | `exit_status::user_panic_reports_an_abnormal_exit_to_the_shell` | 15.13 |
| `arp_watch::arp_watch_decodes_injected_frame` | 19.43 | `ctrl_c::ctrl_c_at_a_bare_prompt_redraws_and_keeps_the_shell_usable` | 15.11 |
| `expose_corrupt_hdd::a_corrupt_partition_table_exposes_the_whole_drive_for_rescue` | 17.79 | `boot_and_shell::shell_ls_and_unknown_command` | 15.05 |
| `kstats_mem::load_run_kill_cycles_do_not_erode_used_memory` | 17.75 | `ipc_bulk::bulk_data_transfer_echo_and_limits_roundtrip` | 15.04 |
| `exit_status::a_waiting_parent_always_receives_the_clean_seven` | 17.30 | `ps_listing::ps_reports_a_process_table_over_ipc` | 15.02 |
| `ctrl_c::ctrl_c_spares_background_jobs` | 17.17 | `ipc_ping::shell_ping_capability_data_roundtrip` | 15.02 |
| `shell_background::a_built_in_announces_that_the_background_request_is_ignored` | 16.05 | `waitset_or::shell_waitset_or_composition_roundtrip` | 15.00 |
| `blockd_refresh_witness::a_refresh_after_a_vendor_face_table_edit_recurates_the_namespace` | 16.04 | `exit_status::exiting_from_main_is_silently_clean` | 14.99 |
| `ipc_random::shell_random_capability_ipc_roundtrip` | 15.99 | `boot_and_shell::shell_echoes_keystrokes` | 14.99 |
| `ted_edit::ted_shrink_save_rewrites_the_shorter_file` | 15.78 | `kstats_mem::mem_reports_a_memory_snapshot_over_ipc` | 14.99 |
| `shell_background::repeated_background_launches_are_each_reaped_exactly_once` | 15.69 | `ps_listing::ps_lists_live_processes_by_their_program_names` | 14.97 |
| `ted_edit::ted_creates_a_new_file_from_a_bare_name` | 15.68 | `ata_read::ata_client_reads_the_mbr_through_atad` | 14.96 |
| `ted_edit::ted_save_as_names_an_untitled_buffer` | 15.61 | `ps_listing::ps_accounts_each_process_stack_to_its_own_group` | 14.96 |
| `kstats_mem::repeated_mem_runs_report_stable_used_bytes` | 15.59 | `elf_arguments::elf_app_receives_arguments_from_kernel` | 14.94 |
| `ted_edit::ted_creates_saves_and_exits_with_clean_buffer` | 15.59 | `ps_listing::ps_reports_vfsd_as_a_thin_mount_table_daemon` | 14.92 |
| `future_ownership::a_dying_group_forfeits_its_abandoned_futures_and_the_pool_stays_usable` | 15.58 | `ata_identify::atad_reports_the_boot_disk_identity_at_boot` | 14.86 |
| `ipc_wake_latency::parked_services_run_before_the_waking_burst_completes` | 15.57 | `boot_and_shell::shell_banner_and_prompt` | 14.82 |
| `ted_edit::ted_quit_prompt_answers_no_then_yes` | 15.53 | `ne2k_smoke::ne2k_cli_drives_the_stack_on_one_guest` | 8.09 |
| `ted_edit::ted_quit_through_the_save_as_prompt_exits_after_saving` | 15.52 | `ip_slirp::ip_ping_crosses_slirp` | 7.01 |
| `ipc_vfs::shell_vfs_filesystem_roundtrip` | 15.51 | `ip_inject::injected_frames_are_answered_by_ipd` | 5.67 |
| `shell_background::a_foreground_command_still_waits_while_a_background_job_runs` | 15.40 | `arp_slirp::arp_who_has_answers_through_slirp` | 5.38 |
| `elf_launch_failure::launching_a_corrupt_elf_fails_at_the_syscall_with_a_kernel_diagnostic` | 15.39 | `ne2k_watch::ne2k_watch_receives_pushed_frame` | 5.21 |
| `ctrl_c::ctrl_c_after_a_partial_line_discards_the_line` | 15.35 | `boot_and_shell::kernel_boots` | 2.96 |
| `exit_status::explicit_exit_reports_its_code_to_the_shell` | 15.33 | `disk_boot::a_mounted_root_without_a_shell_falls_back_to_the_rom_shell` | 0.00 |
| `elf_launch_failure::launching_a_text_file_reports_a_failed_launch_and_the_system_stays_up` | 15.33 | `disk_boot::boot_disk_mounts_the_root_and_hands_off_the_disk_shell` | 0.00 |
| `exit_status::malformed_exit_request_reports_usage_and_the_shell_survives` | 15.28 | `disk_boot::the_built_boot_disk_boots_to_the_disk_shell` | 0.00 |
| `shell_background::background_launch_announces_start_and_finishes_on_a_later_prompt` | 15.20 | | |

### x86_32 — wall 11 m 47 s (707 s), test sum 706.9 s

59 tests: 58 spawn a VM (`ipc_vfs` skips on this arch), 3 of them boot a
second VM in-test → **≈61 VM boots**. 0 failures. The floor for a simple
prompt-driven test is ≈11.8 s.

| Test | s | Test | s |
|---|---:|---|---:|
| `vfs_hdd_flow::register_format_and_carve_directories_on_the_hard_disk_volume` | 33.35 | `exit_status::user_panic_reports_an_abnormal_exit_to_the_shell` | 12.06 |
| `partitioned_hdd_flow::slices_are_first_class_devices_on_a_partitioned_disk` | 29.39 | `shell_background::background_launch_announces_start_and_finishes_on_a_later_prompt` | 12.04 |
| `fat_partition_flow::fat_partition_mounts_by_autodetection_and_survives_reboot` | 28.93 | `ctrl_c::ctrl_c_at_a_bare_prompt_redraws_and_keeps_the_shell_usable` | 12.00 |
| `shell_background::a_panicking_background_job_is_reported_at_a_later_prompt` | 22.05 | `shell_command_arguments::shell_forwards_command_arguments_to_elf_app` | 11.99 |
| `arp_watch::arp_watch_decodes_injected_frame` | 16.82 | `ipc_ping::shell_ping_capability_data_roundtrip` | 11.91 |
| `expose_corrupt_hdd::a_corrupt_partition_table_exposes_the_whole_drive_for_rescue` | 14.61 | `waitset_or::shell_waitset_or_composition_roundtrip` | 11.91 |
| `kstats_mem::load_run_kill_cycles_do_not_erode_used_memory` | 14.25 | `ps_listing::ps_reports_a_process_table_over_ipc` | 11.89 |
| `ctrl_c::ctrl_c_spares_background_jobs` | 13.91 | `disk_boot::the_built_boot_disk_boots_to_the_disk_shell` | 11.88 |
| `exit_status::a_waiting_parent_always_receives_the_clean_seven` | 13.75 | `kstats_mem::mem_reports_a_memory_snapshot_over_ipc` | 11.87 |
| `ipc_random::shell_random_capability_ipc_roundtrip` | 13.48 | `ps_listing::ps_reports_vfsd_as_a_thin_mount_table_daemon` | 11.86 |
| `ted_edit::ted_shrink_save_rewrites_the_shorter_file` | 13.04 | `exit_status::exiting_from_main_is_silently_clean` | 11.82 |
| `blockd_refresh_witness::a_refresh_after_a_vendor_face_table_edit_recurates_the_namespace` | 12.94 | `ata_read::ata_client_reads_the_mbr_through_atad` | 11.82 |
| `shell_background::a_built_in_announces_that_the_background_request_is_ignored` | 12.83 | `ipc_bulk::bulk_data_transfer_echo_and_limits_roundtrip` | 11.82 |
| `ted_edit::ted_save_as_names_an_untitled_buffer` | 12.64 | `boot_and_shell::shell_ls_and_unknown_command` | 11.80 |
| `shell_background::repeated_background_launches_are_each_reaped_exactly_once` | 12.57 | `ps_listing::ps_lists_live_processes_by_their_program_names` | 11.79 |
| `future_ownership::a_dying_group_forfeits_its_abandoned_futures_and_the_pool_stays_usable` | 12.47 | `ps_listing::ps_accounts_each_process_stack_to_its_own_group` | 11.78 |
| `ted_edit::ted_creates_a_new_file_from_a_bare_name` | 12.46 | `boot_and_shell::shell_echoes_keystrokes` | 11.77 |
| `ted_edit::ted_creates_saves_and_exits_with_clean_buffer` | 12.43 | `elf_arguments::elf_app_receives_arguments_from_kernel` | 11.76 |
| `ted_edit::ted_quit_through_the_save_as_prompt_exits_after_saving` | 12.38 | `ata_identify::atad_reports_the_boot_disk_identity_at_boot` | 11.76 |
| `ipc_wake_latency::parked_services_run_before_the_waking_burst_completes` | 12.34 | `boot_and_shell::shell_banner_and_prompt` | 11.65 |
| `kstats_mem::repeated_mem_runs_report_stable_used_bytes` | 12.32 | `ne2k_smoke::ne2k_cli_drives_the_stack_on_one_guest` | 5.25 |
| `ted_edit::ted_quit_prompt_answers_no_then_yes` | 12.30 | `ip_slirp::ip_ping_crosses_slirp` | 4.20 |
| `elf_launch_failure::launching_a_corrupt_elf_fails_at_the_syscall_with_a_kernel_diagnostic` | 12.27 | `ip_inject::injected_frames_are_answered_by_ipd` | 3.02 |
| `exit_status::malformed_exit_request_reports_usage_and_the_shell_survives` | 12.15 | `ne2k_watch::ne2k_watch_receives_pushed_frame` | 2.57 |
| `shell_background::a_foreground_command_still_waits_while_a_background_job_runs` | 12.14 | `arp_slirp::arp_who_has_answers_through_slirp` | 2.56 |
| `ctrl_c::ctrl_c_after_a_partial_line_discards_the_line` | 12.14 | `disk_boot::boot_disk_mounts_the_root_and_hands_off_the_disk_shell` | 1.98 |
| `ctrl_c::ctrl_c_kills_the_running_foreground_app` | 12.13 | `disk_boot::a_mounted_root_without_a_shell_falls_back_to_the_rom_shell` | 1.20 |
| `elf_launch_failure::launching_a_text_file_reports_a_failed_launch_and_the_system_stays_up` | 12.10 | `boot_and_shell::kernel_boots` | 0.54 |
| `ata_write::ata_write_roundtrips_on_the_scratch_slave` | 12.09 | `ipc_vfs::shell_vfs_filesystem_roundtrip` | 0.00 |
| `exit_status::explicit_exit_reports_its_code_to_the_shell` | 12.09 | | |

### Skips and ignored (both arches, per-binary `test result` lines confirm)

- x86_64: `disk_boot` ×3 skipped (the FAT boot-disk flow belongs to the
  x86_32 port). x86_32: `ipc_vfs` skipped (x86_64-only).
- Not run (pre-existing `#[ignore]`): `bench_allocator`, `bench_baseline`,
  `fsbench`, `stress_mixed_preemption`, `ata_stress`; `measure_boot` ran
  separately as above.

## Cost model

- The suite is **boot-dominated on both arches** now (it was x86_64-only in
  11-07): floor 15.0 s x86_64 / 11.8 s x86_32 per boot-lower-bound test.
- The probe tax is the single biggest cost: ~50 NIC-less boots × 10 s ≈
  **500 s of 895.8 s (56 %)** on x86_64 and ~52 × 10 s ≈ **520 s of 706.9 s
  (74 %)** on x86_32. NIC-attached tests (`ne2k_*`, `arp_*`, `ip_*`) pay no
  tax and show it: 2.6–8.1 s x86_64 / 2.6–5.3 s x86_32.
- Beyond the tax the floor is still boot: Step 1 should bring it to ≈4.7 s
  (x86_64) / ≈1.7 s (x86_32); Steps 2–4 (shared sessions) then remove the
  remaining boot legs for shell-only groups.
- Longest real-work tests: `vfs_hdd_flow` 40.0 s, `partitioned_hdd_flow`
  35.6 s, `fat_partition_flow` 35.4 s (x86_64) — reboot/format flows with
  two VMs; `arp_watch` 19.4 s is event-tail waiting, not boot.

## Flakes

| Arch | Test | Signature |
|---|---|---|
| x86_64 | `ctrl_c::ctrl_c_kills_the_running_foreground_app` (33.7 s, FAILED) | Timed out (30 s) waiting for `spinner ready`. Transcript shows the line echoed intact — `spinner` → `Unknown command: spinner` → prompt. A shell VFS name lookup missed the ROM-resident `spinner` app once. Echo was acknowledged (not M2); the sibling test in the same binary that launches `spinner &` passed, and the same test passed on x86_32 minutes later. **Confirmed flake by immediate x86_64 re-run: all 4 `ctrl_c` tests passed (62.9 s).** One occurrence — closest to M9 (volume state), no mechanism claim from a single sample. |

## Step 1 measurement (probe-tax fix, appended)

Same protocol, same host, quiet gate (commit `ad1868a`, kernels rebuilt from
the final tree). The fix: `init` attaches each service's completion future
to a waitset and lets the 100 ms probe sleep bound the watch, so a service
that dies (like `ne2kd` with no NIC) stops the probe loop at once with
`[INIT] <probe> service exited before ready`.

| Metric | before | after | Δ |
|---|---|---|---|
| boot-to-prompt median x86_64 | 14.69 s | **4.33 s** | **−71 %** |
| boot-to-prompt median x86_32 | 11.64 s | **1.74 s** | **−85 %** |
| suite wall x86_64 | 15 m 07 s | **5 m 40 s** | **−63 %** |
| suite wall x86_32 | 11 m 47 s | **3 m 04 s** | **−74 %** |
| test sum x86_64 | 895.8 s | 339.8 s | −62 % |
| test sum x86_32 | 706.9 s | 184.1 s | −74 % |
| simple-test floor x86_64 | ≈15.0 s | ≈4.4–4.6 s | |
| simple-test floor x86_32 | ≈11.8 s | ≈1.8 s | |

- 60 tests per arch now (the new `boot_and_shell::nic_less_boot_reports_the_exited_network_service`
  asserts the exit line and the prompt with a generous 60 s budget).
- 0 flakes in both re-runs, including `ctrl_c` (the Step 0 flake did not recur).
- NIC-attached tests unchanged (`ne2k_smoke` 8.2 s / 5.3 s): a present NIC is
  still probed to `ready`.
- The remaining boot leg is the real service bring-up (~1.4 s init work on
  top of the 2.9 s x86_64 SeaBIOS+stage2 carrier, 0.44 s on x86_32) — what
  Steps 2–4 (shared sessions) remove for shell-only groups.
- `cargo xtask verify` green on the fix (gate + integration both arches +
  boot-test 25/25).
