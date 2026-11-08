use alloc::string::String;
use alloc::vec::Vec;
use crate::args::{MAX_ARGUMENT_BYTES, MAX_ARGUMENT_COUNT};

pub const VERB_SERVICE: &str = "service";
pub const VERB_RUN: &str = "run";
pub const VERB_EXEC: &str = "exec";

pub const INIT_ARGV0: &str = "init";

pub const MAX_PLAN_STEPS: usize = MAX_ARGUMENT_COUNT - 1;
pub const MAX_PLAN_BYTES: usize = MAX_ARGUMENT_BYTES;

pub const ROM_SOURCE_PREFIX: &str = "rom:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Program<'a> {
    Rom(&'a str),
    Vfs(&'a str),
    ImplicitRom(&'a str),
}

pub fn parse_program(token: &str) -> Option<Program<'_>> {
    if token.is_empty() {
        return None;
    }
    if let Some(name) = token.strip_prefix(ROM_SOURCE_PREFIX) {
        if name.is_empty() || name.contains('/') || name.contains(':') {
            return None;
        }
        return Some(Program::Rom(name));
    }
    if let Some(path) = token.strip_prefix('/') {
        if path.is_empty() {
            return None;
        }
        return Some(Program::Vfs(token));
    }
    if token.contains('/') || token.contains(':') {
        return None;
    }
    Some(Program::ImplicitRom(token))
}

#[derive(Debug, Default)]
pub struct BootPlan {
    steps: Vec<String>,
}

impl BootPlan {
    pub fn new() -> BootPlan {
        BootPlan::default()
    }

    pub fn service(mut self, name: &str, probe: &str) -> BootPlan {
        self.add_step(&[VERB_SERVICE, name, probe]);
        self
    }

    pub fn run(mut self, name: &str, arguments: &[&str]) -> BootPlan {
        self.add_step_with_arguments(VERB_RUN, name, arguments);
        self
    }

    pub fn exec(mut self, name: &str, arguments: &[&str]) -> BootPlan {
        self.add_step_with_arguments(VERB_EXEC, name, arguments);
        self
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut blob = Vec::new();
        blob.extend_from_slice(INIT_ARGV0.as_bytes());
        blob.push(0);
        for step in &self.steps {
            blob.extend_from_slice(step.as_bytes());
            blob.push(0);
        }
        assert!(
            blob.len() <= MAX_PLAN_BYTES,
            "boot plan exceeds {MAX_PLAN_BYTES} bytes: {}",
            blob.len()
        );
        blob
    }

    fn add_step_with_arguments(&mut self, verb: &str, name: &str, arguments: &[&str]) {
        let mut tokens: Vec<&str> = Vec::with_capacity(arguments.len() + 2);
        tokens.push(verb);
        tokens.push(name);
        tokens.extend_from_slice(arguments);
        self.add_step(&tokens);
    }

    fn add_step(&mut self, tokens: &[&str]) {
        assert!(
            tokens.len() >= 2,
            "a boot plan step needs a verb and an executable name"
        );
        for token in tokens {
            assert!(!token.is_empty(), "a boot plan token must not be empty");
            assert!(
                !token.contains(' '),
                "a boot plan token must not contain spaces: {token}"
            );
        }
        let name = tokens[1];
        assert!(
            parse_program(name).is_some(),
            "a boot plan program must name a source: rom:<name>, /<path>, or a bare name: {name}"
        );
        assert!(
            self.steps.len() < MAX_PLAN_STEPS,
            "a boot plan must not exceed {MAX_PLAN_STEPS} steps"
        );
        self.steps.push(tokens.join(" "));
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step<'a> {
    Service { name: &'a str, probe: &'a str },
    Run { name: &'a str, arguments: Vec<&'a str> },
    Exec { name: &'a str, arguments: Vec<&'a str> },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PlanError {
    Empty,
    UnknownVerb,
    Malformed,
}

pub fn parse_step(line: &str) -> Result<Step<'_>, PlanError> {
    if line.is_empty() {
        return Err(PlanError::Empty);
    }
    let mut tokens = line.split(' ');
    let verb = tokens.next().unwrap_or("");
    match verb {
        VERB_SERVICE => {
            let name = parse_program_name(&mut tokens)?;
            let probe = tokens.next().ok_or(PlanError::Malformed)?;
            let rest = tokens.count();
            if probe.is_empty() || rest != 0 {
                return Err(PlanError::Malformed);
            }
            Ok(Step::Service { name, probe })
        }
        VERB_RUN => {
            let (name, arguments) = parse_name_and_arguments(&mut tokens)?;
            Ok(Step::Run { name, arguments })
        }
        VERB_EXEC => {
            let (name, arguments) = parse_name_and_arguments(&mut tokens)?;
            Ok(Step::Exec { name, arguments })
        }
        _ => Err(PlanError::UnknownVerb),
    }
}

fn parse_program_name<'a>(
    tokens: &mut core::str::Split<'a, char>,
) -> Result<&'a str, PlanError> {
    let name = tokens.next().ok_or(PlanError::Malformed)?;
    if parse_program(name).is_none() {
        return Err(PlanError::Malformed);
    }
    Ok(name)
}

fn parse_name_and_arguments<'a>(
    tokens: &mut core::str::Split<'a, char>,
) -> Result<(&'a str, Vec<&'a str>), PlanError> {
    let name = parse_program_name(tokens)?;
    let mut arguments: Vec<&'a str> = Vec::new();
    for token in tokens {
        if token.is_empty() {
            return Err(PlanError::Malformed);
        }
        arguments.push(token);
    }
    Ok((name, arguments))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step_lines(blob: &[u8]) -> Vec<&str> {
        core::str::from_utf8(blob)
            .expect("encoded plan is utf8")
            .split('\0')
            .filter(|argument| !argument.is_empty())
            .collect()
    }

    #[test]
    fn encode_leads_with_argv0_and_nul_terminates_every_step() {
        let plan = BootPlan::new().service("vfsd", "VFS").exec("shell", &[]);
        assert_eq!(plan.encode(), b"init\0service vfsd VFS\0exec shell\0");
    }

    #[test]
    fn encode_appends_run_and_exec_arguments_in_order() {
        let plan = BootPlan::new()
            .run("attach", &["ATA"])
            .exec("shell", &["--fast"]);
        assert_eq!(plan.encode(), b"init\0run attach ATA\0exec shell --fast\0");
    }

    #[test]
    fn every_step_kind_survives_the_encode_parse_round_trip() {
        let plan = BootPlan::new()
            .service("vfsd", "VFS")
            .run("mount", &["hdd", "hdd"])
            .run("mkdir", &[])
            .exec("shell", &[]);
        let blob = plan.encode();
        let arguments = step_lines(&blob);
        assert_eq!(arguments[0], INIT_ARGV0);
        assert_eq!(
            parse_step(arguments[1]),
            Ok(Step::Service { name: "vfsd", probe: "VFS" })
        );
        assert_eq!(
            parse_step(arguments[2]),
            Ok(Step::Run { name: "mount", arguments: vec!["hdd", "hdd"] })
        );
        assert_eq!(
            parse_step(arguments[3]),
            Ok(Step::Run { name: "mkdir", arguments: vec![] })
        );
        assert_eq!(
            parse_step(arguments[4]),
            Ok(Step::Exec { name: "shell", arguments: vec![] })
        );
    }

    #[test]
    fn a_full_service_roster_fits_the_argument_budget() {
        let plan = BootPlan::new()
            .service("vfsd", "VFS")
            .service("inputd", "INPUTD")
            .service("ps2d", "PS2KEYBOARD")
            .service("atad", "ATA")
            .service("randomd", "RANDOM")
            .service("echo", "BULKECHO")
            .service("pong", "PING")
            .exec("shell", &[]);
        let blob = plan.encode();
        assert!(blob.len() <= MAX_PLAN_BYTES);
        assert_eq!(
            blob.iter().filter(|byte| **byte == 0).count(),
            9,
            "one nul per step plus argv0"
        );
    }

    #[test]
    fn the_step_cap_matches_the_argument_count_minus_argv0() {
        assert_eq!(MAX_PLAN_STEPS, MAX_ARGUMENT_COUNT - 1);
    }

    #[test]
    fn a_max_length_plan_encodes_within_the_argument_budget() {
        let mut plan = BootPlan::new();
        for _ in 0..MAX_PLAN_STEPS {
            plan = plan.service("vfsd", "VFS");
        }
        assert_eq!(step_lines(&plan.encode()).len(), MAX_PLAN_STEPS + 1);
    }

    #[test]
    #[should_panic(expected = "must not exceed")]
    fn one_step_past_the_cap_panics() {
        let mut plan = BootPlan::new();
        for _ in 0..=MAX_PLAN_STEPS {
            plan = plan.service("a", "b");
        }
    }

    #[test]
    #[should_panic(expected = "must not contain spaces")]
    fn a_space_bearing_token_panics() {
        BootPlan::new().service("two words", "VFS");
    }

    #[test]
    #[should_panic(expected = "must not be empty")]
    fn an_empty_token_panics() {
        BootPlan::new().service("vfsd", "");
    }

    #[test]
    #[should_panic(expected = "exceeds")]
    fn a_plan_past_the_byte_budget_panics() {
        let long_name = alloc::string::String::from_iter(core::iter::repeat('n').take(60));
        let mut plan = BootPlan::new();
        for _ in 0..MAX_PLAN_STEPS {
            plan = plan.service(long_name.as_str(), "VFS");
        }
        let _ = plan.encode();
    }

    #[test]
    fn parse_accepts_a_run_with_no_arguments() {
        assert_eq!(
            parse_step("run mkdir"),
            Ok(Step::Run { name: "mkdir", arguments: vec![] })
        );
    }

    #[test]
    fn parse_rejects_malformed_lines() {
        assert_eq!(parse_step(""), Err(PlanError::Empty));
        assert_eq!(parse_step("start vfsd"), Err(PlanError::UnknownVerb));
        assert_eq!(parse_step("service"), Err(PlanError::Malformed));
        assert_eq!(parse_step("service vfsd"), Err(PlanError::Malformed));
        assert_eq!(parse_step("service vfsd VFS extra"), Err(PlanError::Malformed));
        assert_eq!(parse_step("service vfsd  VFS"), Err(PlanError::Malformed));
        assert_eq!(parse_step("run"), Err(PlanError::Malformed));
        assert_eq!(parse_step("exec"), Err(PlanError::Malformed));
        assert_eq!(parse_step("run mount "), Err(PlanError::Malformed));
        assert_eq!(parse_step("run mount  hdd"), Err(PlanError::Malformed));
    }

    #[test]
    fn program_source_recognizes_all_three_forms() {
        assert_eq!(parse_program("rom:vfsd"), Some(Program::Rom("vfsd")));
        assert_eq!(parse_program("/shell"), Some(Program::Vfs("/shell")));
        assert_eq!(parse_program("vfsd"), Some(Program::ImplicitRom("vfsd")));
    }

    #[test]
    fn program_source_refuses_tokens_that_name_no_coherent_source() {
        for token in [
            "", "rom:", "rom:/vfsd", "rom:at/ad", "rom:disk:1", "/", "hdd/shell", "x:y",
        ] {
            assert_eq!(parse_program(token), None, "{token} names no program");
        }
    }

    #[test]
    fn sourced_program_names_survive_the_encode_parse_round_trip() {
        let plan = BootPlan::new()
            .service("rom:vfsd", "VFS")
            .run("rom:mount", &["ata0d0p0", "/"])
            .exec("/shell", &[]);
        let blob = plan.encode();
        let arguments = step_lines(&blob);
        assert_eq!(
            parse_step(arguments[1]),
            Ok(Step::Service { name: "rom:vfsd", probe: "VFS" })
        );
        assert_eq!(
            parse_step(arguments[2]),
            Ok(Step::Run { name: "rom:mount", arguments: vec!["ata0d0p0", "/"] })
        );
        assert_eq!(
            parse_step(arguments[3]),
            Ok(Step::Exec { name: "/shell", arguments: vec![] })
        );
    }

    #[test]
    fn bare_names_decode_as_implicit_rom_programs() {
        let Ok(Step::Exec { name, .. }) = parse_step("exec shell") else {
            panic!("bare names stay valid implicit-rom steps");
        };
        assert_eq!(parse_program(name), Some(Program::ImplicitRom("shell")));
    }

    #[test]
    fn parse_rejects_a_step_whose_program_token_names_no_source() {
        assert_eq!(parse_step("run /"), Err(PlanError::Malformed));
        assert_eq!(parse_step("service rom: VFS"), Err(PlanError::Malformed));
        assert_eq!(parse_step("exec a/b"), Err(PlanError::Malformed));
    }

    #[test]
    #[should_panic(expected = "must name a source")]
    fn the_builder_refuses_a_rom_prefix_without_a_name() {
        BootPlan::new().service("rom:", "VFS");
    }

    #[test]
    #[should_panic(expected = "must name a source")]
    fn the_builder_refuses_a_bare_slash_as_a_program() {
        BootPlan::new().exec("/", &[]);
    }
}
