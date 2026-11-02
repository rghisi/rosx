use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::arch::ArchSpec;

#[derive(Debug, PartialEq, Eq)]
pub struct Violation {
    pub mnemonic: String,
    pub occurrences: usize,
}

pub fn audit_disassembly(text: &str) -> Vec<Violation> {
    let mut offenders: BTreeMap<String, usize> = BTreeMap::new();
    for line in text.lines() {
        let Some((bytes, mnemonic)) = instruction_of(line) else {
            continue;
        };
        if !is_486_safe(bytes, mnemonic) {
            *offenders.entry(mnemonic.to_string()).or_insert(0) += 1;
        }
    }
    offenders
        .into_iter()
        .map(|(mnemonic, occurrences)| Violation { mnemonic, occurrences })
        .collect()
}

fn instruction_of(line: &str) -> Option<(&str, &str)> {
    let mut fields = line.split('\t');
    let address = fields.next()?.trim();
    let digits = address.strip_suffix(':')?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = fields.next()?.trim();
    if bytes.is_empty() {
        return None;
    }
    let mut words = fields.next()?.trim().split_whitespace();
    let mut mnemonic = words.next()?;
    while is_instruction_prefix(mnemonic) {
        mnemonic = words.next()?;
    }
    Some((bytes, mnemonic))
}

fn is_instruction_prefix(word: &str) -> bool {
    matches!(
        word,
        "lock" | "rep" | "repe" | "repz" | "repne" | "repnz" | "data16" | "addr16" | "bnd"
    )
}

fn is_486_safe(bytes: &str, mnemonic: &str) -> bool {
    if mnemonic == "tzcnt" {
        return bytes.split_whitespace().next() == Some("f3");
    }
    if (mnemonic.len() == 2 || mnemonic.len() == 3) && mnemonic.starts_with('j') {
        return true;
    }
    if (mnemonic.len() == 4 || mnemonic.len() == 5) && mnemonic.starts_with("set") {
        return true;
    }
    if SIZED_BASES.contains(&mnemonic) || EXACT_486_MNEMONICS.contains(&mnemonic) {
        return true;
    }
    if mnemonic.len() > 1 {
        let (base, suffix) = mnemonic.split_at(mnemonic.len() - 1);
        if matches!(suffix, "b" | "w" | "l") && SIZED_BASES.contains(&base) {
            return true;
        }
    }
    false
}

const SIZED_BASES: &[&str] = &[
    "adc", "add", "and", "btc", "btr", "bts", "bt", "call", "cmp", "dec", "div", "idiv", "imul",
    "inc", "in", "ins", "mul", "neg", "not", "or", "out", "pop", "push", "rcl", "rcr", "rol",
    "ror", "sar", "sbb", "shl", "shr", "shld", "shrd", "sub", "test", "xchg", "xor",
];

const EXACT_486_MNEMONICS: &[&str] = &[
    "aaa", "aad", "aam", "aas", "arpl", "bsf", "bsr", "bswap", "bound", "cbw", "cdq", "cltd",
    "clc", "cld", "cli", "cmc", "cmpxchg", "cwde", "cwtl", "daa", "das", "enter", "hlt",
    "int", "int1", "int3", "into", "iret", "iretd", "lcall", "leave", "lfs", "lgdt", "lgdtl",
    "lgs", "lidt", "lidtl", "ljmp", "lldt", "lmsw", "lret", "lss", "ltr", "lea", "mov", "movb",
    "movl", "movw", "movsb", "movsd", "movsl", "movsw", "movsbl", "movsbw", "movswl", "movzbl",
    "movzwl", "nop", "pause",
    "popa", "popad", "popf", "popfl", "pusha", "pushad", "pushf", "pushfl", "ret", "sahf",
    "scasb", "scasd", "scasl", "scasw", "sgdt", "sgdtl", "sidt", "sidtl", "sldt", "smsw",
    "stosb", "stosd", "stosl", "stosw", "stc", "std", "sti", "str", "verw", "wait", "xadd",
    "xlat", "xlatb", "clts", "invd", "wbinvd", "loop", "loope", "loopz", "loopne", "loopnz",
    "jecxz", "jrcxz", "lodsb", "lodsw", "lodsl", "lodsd", "cmpsb", "cmpsw", "cmpsl", "cmpsd",
];

pub fn run_check_isa(spec: &ArchSpec, user_elfs: &[&str]) -> Result<Vec<String>, String> {
    let objdump = on_path("objdump").ok_or_else(|| {
        "objdump not found on PATH; install binutils (sudo apt install binutils)".to_string()
    })?;
    let mut failures = Vec::new();
    let mut audited = vec![spec.kernel_elf("release")];
    let release_dir = spec.user_release_dir();
    audited.extend(user_elfs.iter().map(|name| release_dir.join(name)));

    for elf in &audited {
        let output = Command::new(&objdump)
            .arg("-d")
            .arg(elf)
            .output()
            .map_err(|err| format!("failed to run objdump on {}: {err}", elf.display()))?;
        if !output.status.success() {
            return Err(format!(
                "objdump failed on {} (missing? run `cargo xtask build {}` first)",
                elf.display(),
                spec.name
            ));
        }
        let violations = audit_disassembly(&String::from_utf8_lossy(&output.stdout));
        if !violations.is_empty() {
            let detail = violations
                .iter()
                .map(|v| format!("{} x{}", v.mnemonic, v.occurrences))
                .collect::<Vec<_>>()
                .join(", ");
            failures.push(format!("{}: {detail}", elf.display()));
        }
    }
    Ok(failures)
}

pub const STAGE_FILES: [&str; 4] = ["stage1", "stage2", "fat_stage1_5", "stage2_eltorito"];

pub fn audit_stage_disassembly(text: &str) -> Vec<Violation> {
    let mut offenders: BTreeMap<String, usize> = BTreeMap::new();
    for line in text.lines() {
        let Some((_bytes, mnemonic)) = instruction_of(line) else {
            continue;
        };
        if is_post_486(mnemonic) {
            *offenders.entry(mnemonic.to_string()).or_insert(0) += 1;
        }
    }
    offenders
        .into_iter()
        .map(|(mnemonic, occurrences)| Violation { mnemonic, occurrences })
        .collect()
}

fn is_post_486(mnemonic: &str) -> bool {
    mnemonic.starts_with("cmov")
        || matches!(
            mnemonic,
            "cmpxchg8b"
                | "cpuid"
                | "rdtsc"
                | "rdmsr"
                | "wrmsr"
                | "syscall"
                | "sysenter"
                | "sysexit"
                | "fxsave"
                | "fxrstor"
        )
}

pub fn run_check_stages(stages_dir: &Path) -> Result<Vec<String>, String> {
    let objdump = on_path("objdump").ok_or_else(|| {
        "objdump not found on PATH; install binutils (sudo apt install binutils)".to_string()
    })?;
    if !stages_dir.is_dir() {
        return Err(format!(
            "no stage directory at {}; run arch/x86-86box/scripts/fetch-grub-legacy.sh first",
            stages_dir.display()
        ));
    }
    let mut failures = Vec::new();
    for name in STAGE_FILES {
        let stage = stages_dir.join(name);
        if !stage.is_file() {
            return Err(format!("missing stage binary {}", stage.display()));
        }
        let output = Command::new(&objdump)
            .args(["-D", "-b", "binary", "-m", "i386"])
            .arg(&stage)
            .output()
            .map_err(|err| format!("failed to run objdump on {}: {err}", stage.display()))?;
        if !output.status.success() {
            return Err(format!("objdump failed on {}", stage.display()));
        }
        let violations = audit_stage_disassembly(&String::from_utf8_lossy(&output.stdout));
        if !violations.is_empty() {
            let detail = violations
                .iter()
                .map(|v| format!("{} x{}", v.mnemonic, v.occurrences))
                .collect::<Vec<_>>()
                .join(", ");
            failures.push(format!("{}: {detail}", stage.display()));
        }
    }
    Ok(failures)
}

pub(crate) fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn objdump_line(bytes: &str, text: &str) -> String {
        format!("  1010ab:\t{bytes:<20}\t{text}")
    }

    fn audit(lines: &[String]) -> Vec<Violation> {
        audit_disassembly(&lines.join("\n"))
    }

    #[test]
    fn header_and_padding_lines_are_not_instructions() {
        let lines = vec![
            String::from(""),
            String::from("/build/rosx-x86_32:     file format elf32-i386"),
            String::from("Disassembly of section .text:"),
            String::from("00101000 <_start>:"),
            objdump_line("90", ""),
        ];
        assert_eq!(audit(&lines), vec![]);
    }

    #[test]
    fn pentium_and_later_mnemonics_are_violations() {
        for mnemonic in ["cmovne", "cmova", "cmpxchg8b", "cpuid", "rdtsc", "rdmsr", "wrmsr"] {
            let found = audit(&[objdump_line("0f 44 c1", &format!("{mnemonic}   %ecx,%eax"))]);
            assert_eq!(found.len(), 1, "{mnemonic} must be rejected");
            assert_eq!(found[0].mnemonic, mnemonic);
        }
    }

    #[test]
    fn fpu_sse_mmxt_and_guard_mnemonics_are_violations() {
        for mnemonic in [
            "fxsave", "fxrstor", "fld", "fmul", "fadd", "movaps", "movss", "addps", "movq",
            "paddw", "pcmpeqb", "syscall", "sysenter", "lfence", "mfence", "rdpmc", "rdshr",
        ] {
            let found = audit(&[objdump_line("0f ae 00", &format!("{mnemonic}   (%eax)"))]);
            assert_eq!(found.len(), 1, "{mnemonic} must be rejected");
        }
    }

    #[test]
    fn kernel_workhorse_mnemonics_are_486_safe() {
        let lines = [
            ("55", "push   %ebp"),
            ("89 e5", "mov    %esp,%ebp"),
            ("f0 0f b1 08", "lock cmpxchg %ecx,(%eax)"),
            ("f0 0f c1 0f", "lock xadd %ecx,(%edi)"),
            ("f0 ff 0f", "lock decl (%edi)"),
            ("f0 83 83 98 05 01 00 0a", "lock addl $0xa,0x10598(%ebx)"),
            ("f3 0f bc c9", "tzcnt  %ecx,%ecx"),
            ("0f bc c9", "bsf    %ecx,%ecx"),
            ("f3 90", "pause"),
            ("90", "nop"),
            ("0f b6 c0", "movzbl %al,%eax"),
            ("0f a4 d0 05", "shld   $0x5,%edx,%eax"),
            ("f3 a5", "rep movsl %ds:(%esi),%es:(%edi)"),
            ("0f b1 08", "cmpxchg %ecx,(%eax)"),
            ("cd 80", "int    $0x80"),
            ("cf", "iret"),
            ("0f 01 1d 00 10 00 00", "lgdtl  0x1000"),
            ("9c", "pushf"),
            ("f7 e1", "mull   %ecx"),
            ("0f ac d0 03", "shrd   $0x3,%edx,%eax"),
            ("0f ba 38 01", "btsl   $0x1,(%eax)"),
            ("8d 45 f4", "lea    0xfffffff4(%ebp),%eax"),
            ("e8 00 00 00 00", "call   1010b0 <foo>"),
            ("74 05", "je     1010b2 <bar>"),
            ("0f 9e c0", "setg   %al"),
            ("66 89 10", "movw   %dx,(%eax)"),
            ("ec", "in     $0x21,%al"),
            ("e6 21", "out    %al,$0x21"),
            ("99", "cltd"),
        ];
        let lines: Vec<String> = lines.iter().map(|(b, t)| objdump_line(b, t)).collect();
        assert_eq!(audit(&lines), vec![]);
    }

    #[test]
    fn tzcnt_without_legacy_prefix_is_a_violation() {
        let found = audit(&[objdump_line("0f bc c9", "tzcnt  %ecx,%ecx")]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].mnemonic, "tzcnt");
    }

    #[test]
    fn lzcnt_is_a_violation_even_with_prefix() {
        let found = audit(&[objdump_line("f3 0f bd c9", "lzcnt  %ecx,%ecx")]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].mnemonic, "lzcnt");
    }

    #[test]
    fn occurrence_counts_aggregate_per_mnemonic() {
        let lines = vec![
            objdump_line("0f 44 c1", "cmovne %ecx,%eax"),
            objdump_line("0f 44 d0", "cmovne %edx,%eax"),
            objdump_line("0f a2", "cpuid"),
        ];
        let found = audit(&lines);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].mnemonic, "cmovne");
        assert_eq!(found[0].occurrences, 2);
        assert_eq!(found[1].mnemonic, "cpuid");
        assert_eq!(found[1].occurrences, 1);
    }

    #[test]
    fn prefix_stripped_lock_and_rep_forms_pass_through_to_base_checks() {
        let bad = audit(&[objdump_line("f0 0f c7 08", "lock cmpxchg8b (%eax)")]);
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].mnemonic, "cmpxchg8b");
        let rep_cmov = audit(&[objdump_line("f3 0f 44 c1", "rep cmovne %ecx,%eax")]);
        assert_eq!(rep_cmov[0].mnemonic, "cmovne");
    }

    fn stage_lines(entries: &[(&str, &str)]) -> String {
        entries
            .iter()
            .map(|(b, t)| objdump_line(b, t))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn stage_audit_flags_only_post_486_mnemonics() {
        let clean = stage_lines(&[
            ("eb 3f", "jmp    42 <.L1>"),
            ("89 e5", "mov    %esp,%ebp"),
            ("f3 a5", "rep movsl %ds:(%esi),%es:(%edi)"),
            ("cd 80", "int    $0x80"),
            ("cf", "iret"),
            ("0f b6 c0", "movzbl %al,%eax"),
            ("66 89 10", "movw   %dx,(%eax)"),
        ]);
        assert_eq!(audit_stage_disassembly(&clean), vec![]);

        let dirty = stage_lines(&[
            ("0f 44 c1", "cmovne %ecx,%eax"),
            ("0f 44 d0", "cmovne %edx,%eax"),
            ("0f c7 08", "cmpxchg8b (%eax)"),
            ("0f a2", "cpuid"),
            ("31 c0", "xor    %eax,%eax"),
        ]);
        let found = audit_stage_disassembly(&dirty);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].mnemonic, "cmovne");
        assert_eq!(found[0].occurrences, 2);
        assert_eq!(found[1].mnemonic, "cmpxchg8b");
        assert_eq!(found[2].mnemonic, "cpuid");
    }

    #[test]
    fn stage_audit_tolerates_garbage_decode_of_data_regions() {
        let fpu_and_386_noise = stage_lines(&[
            ("d9 c0", "fld    %st(0),%st"),
            ("de c1", "fmulp  %st,%st(1)"),
            ("66 0f 6f c1", "pavgb  %xmm1,%xmm0"),
            ("df f8", "fnstsw %ax"),
            ("0f 0f c1 00", "nm    %ecx"),
        ]);
        assert_eq!(audit_stage_disassembly(&fpu_and_386_noise), vec![]);
    }

    #[test]
    fn stage_audit_flags_sysenter_rdtsc_and_fxsave_families() {
        for mnemonic in [
            "sysenter",
            "sysexit",
            "rdtsc",
            "rdmsr",
            "wrmsr",
            "fxsave",
            "fxrstor",
            "cmpxchg8b",
            "cpuid",
        ] {
            let line = stage_lines(&[("0f 34", mnemonic)]);
            let found = audit_stage_disassembly(&line);
            assert_eq!(found.len(), 1, "{mnemonic} must be rejected");
        }
    }
}
