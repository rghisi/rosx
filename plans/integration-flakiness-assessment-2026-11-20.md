# Integration-suite flakiness — root causes and per-test value assessment

> **Date:** 2026-11-20 · **Branch:** `integration-flakiness-audit` · **Base:** `origin/main@0d25ecd`
> **Scope:** `tests-integration` on x86_64 and x86_32, as CI runs them (`--test-threads=1`, release kernel via `ROSX_KERNEL_ELF`, pure TCG).

---

## 1. Evidence summary

### CI (last ~100 `ci.yml` runs, 2026-11-16 → 2026-11-20)

| Failure attributed to | Count |
|---|---|
| Integration Tests (x86_64) | 21 |
| Integration Tests (x86_32) | 19 |
| Create Release (unrelated, deterministic — see §5) | 21 |
| Unit Tests / Build Kernel (real code failures at the time) | 3 |

~40 % of all CI runs went red in five days, almost entirely the integration jobs.
Five separate **merge commits landing on main failed their push run** (#177, ctrl-c,
ted, #183, AGENTS-slimdown).

### Same-tree green→red proofs (environment, not code)

| Commit pair (PR head ≡ merge, zero diff) | PR run | Main push run |
|---|---|---|
| `d216ce74` ≡ `65251ed0` (#183) | success (19:10Z Nov 19) | **FAILED Integration x86_32** (22:06Z Nov 19) |
| `0a346030` ≡ `0d7f5135` (#177) | success (08:50Z Nov 19) | **FAILED Integration x86_64** (14:53Z Nov 19) |

### Local reproduction (this host: 4 cores / 7 GiB, QEMU 10.2.1, pure TCG — the
same host class the 2026-11-07 baseline doc measured)

| Run | Result | Wall |
|---|---|---|
| x86_64 full suite, round 1 | 103/103 pass | ~11 min |
| x86_64 full suite, round 2 (`--report-time`) | 103/103 pass | test-sum 949 s |
| x86_32 full suite, round 1 | 102/102 pass | ~14 min |
| x86_32 full suite, round 2 (`--report-time`) | 102/102 pass | test-sum 936 s |

### Local reproduction under injected load (4 CPU burners on 4 cores + the test's own QEMU)

| Case | Result | Signature | Class |
|---|---|---|---|
| `boot_and_shell` ×2 (control) | PASS | — | — |
| `ipc_wake_latency` ×3 | 2 pass / **1 FAIL** | 90 s budget on `[IPC Server] Ping` — session had `no block devices`, `mount failed: Vfs(NotFound)`, `handing over to rom:shell`; `/pong` never loaded | disk-vanish (M6/M9) |
| `kstats_mem` ×2 | 1 pass / **1 FAIL** | `Unknown command: mem` — ROM-shell fallback: `mem` exists only on the disk volume | disk-vanish (M6/M9) |
| `ted_edit` ×1 (6 tests) | **FAIL** (≥2) | `keystroke '\n' never produced "^S Save" after 4 attempts` | input path (M2) |
| `ne2k_pair` ×3 | 2 pass / **1 FAIL** | 45 s on `[NE2KD] ISA NIC…` — same `no block devices` → `rom:shell`; `ne2kd` is a disk app | disk-vanish (M6/M9) |
| `ip_pair` ×2 | 1 pass / **1 FAIL** | `ping: 2 transmitted, 0 received, 100% loss` — boot healthy, frames genuinely lost on the UDP medium | UDP loss (M3) |
| `arp_pair` ×1 | **FAIL** | `requester got no answer` after all retries; binary took 476 s (8× its clean time) — responder's `arp respond 25` wall-window likely expired mid-run | UDP loss + wall budgets (M3/M4) |
| `ctrl_c` ×1 | **FAIL** | `Unknown command: exiter` — ROM-shell fallback again | disk-vanish (M6/M9) |

**7 failures / 15 runs under injected contention; 0 failures / 206 sessions across
four quiet full suites.** The dominant local failure class (4/7) is the same one
the CI `ne2k_pair` log showed on Nov 20: **the FAT master drive intermittently
fails to appear at boot under contention, and the boot plan's fallback masks it
into `rom:shell`** — after which every disk-app needle fails confusingly
(`Unknown command: mem|ne2k|exiter`, `[IPC Server] *` never printed, mount NotFound).
A follow-up `ata_identify` ×6 probe under burners-only load saw the drive 6/6 —
the trigger needs the full contention mix (burners + concurrent QEMU processes),
matching CI's runner profile rather than raw CPU starvation.

---

## 2. Flake mechanisms

### M1 — Output-window race in `run_command` / window snapshots (test bug)
`QemuSession::run_command` breaks as soon as the predicate matches a *delta*
substring and the caller then asserts on the window snapshot. Debugcon output
arrives in chunks that split lines mid-word, so if the predicate matches the
*head* of the line the snapshot clips its tail.
**Captured in the wild:** CI run 44116127470 (`ip_pair`, Nov 17): the completion
needle was `"ping: "`; the snapshot ended at `2 receive` while `d, 0% loss` was
still in flight — **the ping had actually succeeded** (both echo replies are in
the transcript). The current needle `"% loss"` sits at the line end and is
happen-safely correct; the fragile *shape* persists wherever predicate ≠ needle.

### M2 — Keyboard echo-ack false positives / duplicate resends (input path)
`send_key_acked` treats *any* occurrence of the typed character in new guest
output as the shell echo — service log lines containing the character satisfy it,
so keys can be fired while the shell isn't ready; when the real echo is merely
slow (> 500 ms under load) the resend duplicates a character *out of order*
behind the still-draining PS/2 queue. Garbled lines then rely on `run_command`'s
retry loop to paper over; direct `send_line` + expect tests have no retry.
Recorded already in the 2026-11-07 baseline (`vfs_hdd_flow` "zero echo" flake;
echo-ack `e5418b6` reduced, not eliminated — the i8042 16-byte queue still
overflows under TCG stalls).

### M3 — UDP multicast medium is lossy and host-dependent
`ne2k_isa` sessions share a `socket,mcast=230.0.0.1:<port>` netdev — plain UDP
multicast on the runner. No flow control, no retransmit, membership is silent
if the runner's routes refuse multicast. `ne2k_pair` sends **exactly one frame**
and waits 30 s; the injector tests (`*_watch`, `ip_inject`) re-send in a loop and
tolerate loss. CI-harvested evidence: `ip_pair` Nov 17 failed on an M1 harness bug
while crossing this medium; locally under contention the medium itself failed:
`ip_pair` with a genuine `2 transmitted, 0 received, 100% loss` on a healthy boot,
and `arp_pair` starved to 476 s with no answer.

### M4 — Wall-clock budgets vs virtual time
Fixed host sleeps (`arp_pair` 6 s + 4 s, `ip_pair` 3 s, `arp_watch` 5 s), the
`spinner tick 20` needle inside a 30 s budget, and PIT-driven guest pacing all
slip when the TCG thread is descheduled. Mostly absorbed by generous caps today;
contributes secondarily when combined with M2/M3.

### M5 — Brittle content gates turn one hiccup into a red run
Exact line counts (`ipc_random`: exactly 259 of three prefixes), forbidden
substrings over the whole window (`"fail"`, `"Panic"`), memory-spread magic
thresholds (`kstats_mem`: spread < 6 KiB over 5 runs), and output-position
ordering (`ipc_wake_latency`: PONG strictly between BEGIN and SENT). Any dropped
keystroke or extra daemon line flips these from pass to fail even when the
behaviour under test is correct.

### M6 — Boot-plan masking amplifies every media hiccup
The log-and-continue policy means a session whose FAT master disk is missing
falls back to `rom:shell`; every disk-side needle then times out with a confusing
message. **Captured:** CI run 44693683599 (`ne2k_pair`, Nov 20) — the guest
booted with zero block devices (`no block devices`, `mount failed:
Vfs(NotFound)`), init fell back to the ROM shell, and `ne2k mac` died as
`Unknown command: ne2k`. My under-load runs reproduced the identical transcript
on a clean tree, so that CI instance belongs to the M9 class, not (only) to the
branch fixture.

### M9 — The boot-time disk intermittently vanishes under contention (root flake)
`atad` probes each drive exactly once at daemon start (`probe_drives`), and a
failed `probe()` is swallowed (`if let Ok(identity)`): the drive simply does not
exist for the whole session, with no `[ATAD] ata0 drive0 ready` line, no error,
no re-probe ever. Under injected load the probe intermittently returns an error
for a drive QEMU definitely attached, the session lands on `rom:shell` via M6,
and four of the seven load failures above followed. The register-level reason
(0xFF float vs Busy cap vs signature) is invisible because the fault is dropped
— **first fix is observability: print the `DriveFault`**, then a bounded
re-probe (e.g. re-scan on the first `Probe`/`Read` that names a missing drive,
or N boot re-probes over a couple of seconds). This is the single highest-value
kernel-side fix for CI green: ~43 disk-boot sessions per suite run each roll this
dice today.

### M7 — cargo fail-fast turns one flake into a whole-job red
The first failing test binary aborts the remaining ~8 binaries: half the suite
never reports, diagnosis gets harder, and the run is retried from zero.

### M8 — Cost background: the 10 s `ne2kd` probe tax (not a flake, an amplifier)
`ne2kd` exits when no NIC is present; init's service probe burns its full
100×100 ms budget before `exec /shell`. Every NIC-less session pays it: the
per-session floor is ~15 s on **both** arches (was 3.9 s/0.8 s in the Nov-07
baseline). Across ~43 sessions that is ~7 minutes of pure stall per x86_64 run
(and doubles every wall-clock assertion's exposure window). `ne2k_smoke` (NIC
present) boots and finishes in 8.2 s — the delta is the tax.

---

## 3. Per-test assessment

Cost = measured single-session wall on this host (x86_64 round 2; `≈` for x86_32
within ±5 s). Boot floor ~15 s includes the M8 tax.
Flake exposure: structural (mechanisms §2), *italic* = proven in CI or docs.
Value: what production behaviour is guarded, and what would go unguarded.

| # | Test | Guards | Cost | Overlap | Flake exposure | Verdict |
|---|---|---|---|---|---|---|
| 1 | `boot_and_shell::kernel_boots` | boot canary, memory banner | 3 s | subsumed by every test's prompt wait | Low | **Keep** (diagnostic canary, cheapest) |
| 2 | `boot_and_shell::shell_banner_and_prompt` | shell banner + prompt | 14 s | overlaps #3 | Low | **Keep** (fold into #3 if shaving) |
| 3 | `boot_and_shell::shell_echoes_keystrokes` | PS/2→ps2d→inputd→shell echo (only full input smoke) | 15 s | none | Med (M2) | **Keep** |
| 4 | `boot_and_shell::shell_ls_and_unknown_command` | CLI name resolution via VFS | 15 s | `ipc_vfs` (deeper), ted tests | Low | **Keep** |
| 5 | `ctrl_c::ctrl_c_kills_the_running_foreground_app` | Ctrl+C chain ps2d→inputd→shell→kernel | 15 s | none | Med (M2, M4) | **Keep** |
| 6 | `ctrl_c::ctrl_c_at_a_bare_prompt_redraws_and_keeps_the_shell_usable` | bare-prompt Ctrl+C | 15 s | none | Low-Med | **Keep** |
| 7 | `ctrl_c::ctrl_c_after_a_partial_line_discards_the_line` | line-buffer kill | 15 s | none | Low-Med | **Keep** |
| 8 | `ctrl_c::ctrl_c_spares_background_jobs` | signal scoping to fg job | 17 s | none | **High (M4: `spinner tick 20` in 30 s)** | **Keep, re-needle** (replace tick-count needle with "spinner still alive" marker) |
| 9 | `disk_boot::boot_disk_mounts_the_root_and_hands_off_the_disk_shell` (x86_32) | FAT root mount, disk-shell hand-off | ~4 s | `the_built_boot_disk…` | Low | **Keep** |
| 10 | `disk_boot::the_built_boot_disk_boots_to_the_disk_shell` (x86_32) | the *shipped* image boots | 13 s | overlaps #9; only test of the built artifact | Low | **Keep** |
| 11 | `disk_boot::a_mounted_root_without_a_shell_falls_back_to_the_rom_shell` (x86_32) | hand-off fallback | 2 s | none | Low | **Keep** |
| 12 | `elf_arguments::elf_app_receives_arguments_from_kernel` | kernel argv delivery | 15 s | #13 at higher layer | Low | **Keep** |
| 13 | `shell_command_arguments::shell_forwards_command_arguments_to_elf_app` | shell→kernel arg forwarding | 15 s | #12 | Low | **Keep** |
| 14 | `exit_status::exiting_from_main_is_silently_clean` | silent Clean(0) | 15 s | none | Low | **Keep** |
| 15 | `exit_status::explicit_exit_reports_its_code_to_the_shell` | Exit syscall report | 15 s | none | Low | **Keep** |
| 16 | `exit_status::a_waiting_parent_always_receives_the_clean_seven` ×10 | status-in-completion-future race | 17 s | #15 | Low | **Keep** (targeted race canary) |
| 17 | `exit_status::user_panic_reports_an_abnormal_exit_to_the_shell` | panic→Panicked redirect | 15 s | none | Low | **Keep** |
| 18 | `exit_status::malformed_exit_request_reports_usage_and_the_shell_survives` | arg validation + shell survival | 15 s | none | Low | **Keep** |
| 19 | `future_ownership::a_dying_group_forfeits_its_abandoned_futures_and_the_pool_stays_usable` | group-owned future pool | 16 s | none | Low | **Keep** |
| 20 | `ipc_bulk::bulk_data_transfer_echo_and_limits_roundtrip` | bulk data family, budgets, refund | 15 s | none | Med (M5 windows) | **Keep** |
| 21 | `ipc_ping::shell_ping_capability_data_roundtrip` | message+data pair, sizes | 15 s | `ipc_bulk` partial | Med (M5; historic window race #135) | **Keep** |
| 22 | `ipc_random::shell_random_capability_ipc_roundtrip` | 259-iter IPC churn, handle recycle | 16 s | #20/#21 partial | Med (M5 exact counts + forbidden words) | **Keep**, relax exact-count/forbidden gates |
| 23 | `ipc_vfs::shell_vfs_filesystem_roundtrip` (x86_64) | 24-op VFS semantics sweep | 16 s | #4, disk tests | Med (M5) | **Keep** (densest VFS gate) |
| 24 | `ipc_wake_latency::parked_services_run_before_the_waking_burst_completes` | wake-vs-waker scheduling | 16 s | none | **High (M5 position ordering)** | **Keep**, re-design: assert interleaving exists, not byte positions |
| 25 | `kstats_mem::mem_reports_a_memory_snapshot_over_ipc` | kstatsd surface | 15 s | `ps_listing` | Low | **Keep** |
| 26 | `kstats_mem::repeated_mem_runs_report_stable_used_bytes` | leak/erosion gate (caught the real wake race) | 16 s | none | Med (M5 magic 6 KiB) | **Keep**, re-derive threshold or trend-based |
| 27 | `ps_listing::…` ×4 | ps table, names, vfsd thinness, per-group stacks | 60 s (4 boots) | mutual | Low-Med | **Keep content, consolidate to 1 session** (−45 s) |
| 28 | `waitset_or::shell_waitset_or_composition_roundtrip` | waitset OR + negative paths | 15 s | none | Low-Med | **Keep** |
| 29 | `ata_identify::atad_reports_the_boot_disk_identity_at_boot` | atad boot probe banner | 15 s | none (M6 tripwire) | Low | **Keep** |
| 30 | `ata_read::ata_client_reads_the_mbr_through_atad` | vendor-face read + MBR decode | 15 s | `fat_partition_flow` partial | Low | **Keep** |
| 31 | `ata_write::ata_write_roundtrips_on_the_scratch_slave` | slave-drive writeback | 15 s | `blockd_refresh_witness` reuses it | Low | **Keep** |
| 32 | `ata_stress` (ignored) | chunked read stress | — | — | — | Keep ignored |
| 33 | `ata_env_spike::qemu_exposes_the_boot_disk_on_the_primary_ide_channel` | *QEMU's* IDE + monitor PIO tooling | 20 s | nothing in OS scope | Low | **Exclude candidate** — guards the emulator, not RosX; move to boot-test tooling if kept |
| 34 | `vfs_hdd_flow::…` (2 sessions) | register/format/mount/cp/rm + persistence across reboot | 40 s | flagship; `partitioned_hdd_flow` on slices | Med (M2/M6; historic *zero-echo*) | **Keep** (core value) |
| 35 | `partitioned_hdd_flow::…` (2 sessions) | slices first-class, Busy guards, reboot | 36 s | distinct from #34 | Med | **Keep** |
| 36 | `fat_partition_flow::…` (2 sessions) | FAT autodetect + mtools host cross-check | 36 s | unique cross-validator | Med | **Keep** |
| 37 | `expose_corrupt_hdd::…` | corrupt MBR → whole-drive rescue | 18 s | unique | Med | **Keep** |
| 38 | `blockd_refresh_witness::…` | table-edit refresh semantics | 16 s | unique | Low-Med | **Keep** |
| 39 | `ne2k_smoke::…` | ne2kd CLI, tx counters | 8 s | unique cheap | Low | **Keep** |
| 40 | `ne2k_watch::…` | injected frame → IRQ delivery | 5 s | unique receive path, retry-tolerant | Med (M3) | **Keep** |
| 41 | `ne2k_pair::frame_crosses_the_wire_between_two_ne2k_isa_guests` | frame guest→wire→guest, one shot | 12 s | = #39 tx + #40 rx + #44 | ***High (M3 single UDP frame; CI-failed Nov 20)*** | **Exclude candidate** — or rework into inject-until-witnessed with ≤30 s cap |
| 42 | `arp_slirp::…` | ARP against slirp's real gateway | 5 s | unique real peer | Low-Med | **Keep** |
| 43 | `arp_watch::…` | injected who-has decode | 20 s | unique | Med (M3, fixed 5 s sleep) | **Keep**, replace boot sleep with prompt-gated injection |
| 44 | `arp_pair::…` | respond daemon + who-has + announce + watch, 2 guests | 37 s | phases covered by #42/#43/#46 | ***High (M2+M3+M4; CI-failed Nov 17; reproduced under load Nov 20: `requester got no answer`)*** | **Exclude candidate** — fold `announce` into #43, `respond` honesty into #46; keep only if a two-guest story is required |
| 45 | `ip_slirp::…` | full L3 vs real slirp peer: link, config, ping, neigh | 7 s | unique real peer | Med (M2 typing) | **Keep** (high value per second) |
| 46 | `ip_inject::…` | ipd answers ARP + ICMP, host-witnessed w/ retry | 6 s | unique witness mechanism | Med (M3, retry-tolerant) | **Keep** |
| 47 | `ip_pair::…` | two-guest ping + neigh, 2 boots | 33 s | #46 (ipd answers) + #45 (client+neigh) | ***High (M3; CI-failed Nov 17 by an M1 harness bug; reproduced under load Nov 20 with real `100% loss`)*** | **Exclude candidate** — highest-cost, most-redundant net test; move the `ip neigh` assertion into #45 |
| 48 | `shell_background::…` ×5 | bg launch/reap exactly-once, fg coexistence, built-in note | 79 s (5 boots) | mutual overlap | Med (M2; reap-window substring waits) | **Keep content, consolidate to 2 sessions** (−45 s) |
| 49 | `ted_edit::ted_creates_saves_and_exits_with_clean_buffer` | editor→FAT byte-exact | 16 s | flagship ted gate | ***High (M2: heaviest typing; reproduced under load Nov 20)*** | **Keep** |
| 50 | `ted_edit::ted_shrink_save_rewrites_the_shorter_file` | shrink save (delete-then-recreate) | 17 s | unique data-safety case | High (M2) | **Keep** |
| 51 | `ted_edit::ted_creates_a_new_file_from_a_bare_name` | bare-name path | 16 s | #49 + save-as | High (M2) | **Consolidation candidate** |
| 52 | `ted_edit::ted_save_as_names_an_untitled_buffer` | save-as naming | 16 s | unique-ish | High (M2) | **Keep** |
| 53 | `ted_edit::ted_quit_prompt_answers_no_then_yes` | dirty-buffer quit guard | 16 s | unique | High (M2) | **Keep** |
| 54 | `ted_edit::ted_quit_through_the_save_as_prompt_exits_after_saving` | quit→save-as chained prompt | 16 s | #52+#53 composite | High (M2) | **Consolidation candidate** |
| 55 | `iso_boot::grub_iso_boots_to_shell_prompt` (x86_32) | El Torito GRUB boot — closest to silicon path | 13 s (skips w/o tools) | unique | Low | **Keep** |
| 56 | `bench_alloc` / `bench_allocator` / `bench_baseline` / `fsbench` / `stress_mixed_preemption` / `measure_boot` (all `#[ignore]`) | perf & stress tier, outside CI | — | `bench_alloc` ⊂ `bench_allocator` | n/a in CI | Keep ignored; retire `bench_alloc` (superseded) |

Suite cost today: x86_64 test-sum 949 s (~43 boots), x86_32 936 s. The floor
is ~15 s/boot, of which ~10 s is the M8 ne2kd probe tax.

---

## 4. Recommendations

### Exclude / demote (ordered by payoff)
1. **`ip_pair`** — redundant with `ip_inject` + `ip_slirp`, slowest and flakiest
   net test; move its `ip neigh` assertion into `ip_slirp`. Saves ~33 s + one
   proven CI flake source.
2. **`ne2k_pair`** — single-frame UDP send; coverage equals `ne2k_smoke` (tx) +
   `ne2k_watch` (rx, retry). Exclude, or rework into inject-until-witnessed.
3. **`arp_pair`** — four phases, 10 s of fixed sleeps, two guests; fold `announce`
   into `arp_watch`, respond-honesty into `ip_inject`. Exclude otherwise.
4. **`ata_env_spike`** — exercises QEMU's IDE + the monitor console, no shipped
   code; delete or move to `cargo xtask boot-test` tooling.
5. **`ted_edit` bare-name + quit-through-save-as** — consolidation candidates
   (each is a full boot + heaviest typing in the suite).
6. `bench_alloc` — retire from the ignored tier (superseded by `bench_allocator`).

Net effect: −170 s x86_64 suite, and the three CI-proven flake families shrink
by their two harvested names plus the `arp_pair` phase risk.

### Fix, don't exclude (root causes, in payoff order)
- **M9 (do first):** make atad's boot probe failure visible (print the
  `DriveFault` — it is swallowed today) and re-probe bounded: at daemon start
  over a short window and lazily on the first reference to a missing drive.
  The harness can additionally fail fast with a clear message by waiting for
  `mount ok device=ata0d0p0 dir=/` at session start (every session already
  attaches the FAT master), turning the remaining occurrences into an obvious
  `[ATAD]` diagnostic instead of a delayed `Unknown command:` ghost.
- **M8:** init's `service` step should watch the launched task's completion
  future and stop probing the moment the task dies (the exit-status channel
  already delivers exactly this). Removes ~10 s × ~35 boots ≈ 5–6 min per full
  run on both arches and halves every budget's exposure.
- **M1:** make `run_command` break on line-complete matches (needle followed by
  `\n`) or on the prompt; never assert on a snapshot keyed by a head-of-line
  predicate.
- **M2:** line-level resync only — before each `send_line`, require the prompt;
  on retry, clear the input line (Ctrl-u equivalent / `backspace`×) rather than
  re-firing characters into a draining PS/2 queue.
- **M5:** replace exact counts with ≥/≤ bounds where exactness guards nothing;
  scope forbidden-string scans to the tool's own transcript lines.
- **M7:** run CI suite with `--no-fail-fast` so one flake still yields full
  per-binary truth; keep the job red.
- **M3:** keep one-shot wire tests out of PR tier; retry-based (witnessed)
  injection is the pattern that stays green.

### Separate, deterministic CI bug (§ evidence: 21 failures)
`Create Release` has failed on every push since #154 (Nov 09): the upload now
lists two paths, so the artifact tree is `target/…`, and `gzip -9
rosx-x86_64.img` can never find the file at the workspace root. Last successful
release: 2026-11-09. Fix: `gzip -9 rosx-x86_64/target/rosx/release/rosx-x86_64.img`
(or `working-directory`). Not integration-related, but it doubles the red-signal
noise and trains everyone to ignore red CI — which is what makes real flakes
survive.
