use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use crate::records;
use crate::rust_scan::{self, Comment, CommentKind};

const RATIONALE_LINK_MARKER: &str = "docs/rationale/";
const MAX_RATIONALE_COMMENT_LINES: usize = 6;
const HAZARD_ROOTS: [&str; 5] = ["kernel/", "usrlib/", "apps/", "arch/x86_32/", "arch/x86-generic/"];
const HAZARD_TOKENS: [&str; 4] = ["f32", "f64", "AtomicU64", "AtomicI64"];
const BASELINE_PATH: &str = "xtask/style-baseline.txt";

pub const GATES: [&str; 3] = ["comments", "allow", "unsafe"];

pub type Baseline = BTreeMap<(String, String), usize>;

#[derive(Debug, Default, PartialEq)]
pub struct FileReport {
    pub offending_comment_lines: usize,
    pub allow_attributes: usize,
    pub unsafe_tokens: usize,
    pub hazards: Vec<String>,
    pub rationale_links: BTreeSet<String>,
    pub link_problems: Vec<String>,
}

pub fn rationale_links(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    for (index, _) in text.match_indices(RATIONALE_LINK_MARKER) {
        let rest = &text[index + RATIONALE_LINK_MARKER.len()..];
        let slug_length = rest
            .bytes()
            .take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(*b, b'-' | b'_'))
            .count();
        if slug_length > 0 && rest[slug_length..].starts_with(".md") {
            links.push(format!("{}.md", &rest[..slug_length]));
        }
    }
    links
}

pub fn allow_attribute_count(code: &str) -> usize {
    code.match_indices("allow")
        .filter(|(start, _)| {
            let before = code[..*start].trim_end();
            let opened = before.strip_suffix('[').map(str::trim_end);
            let hash_before = opened.is_some_and(|rest| rest.ends_with('#') || rest.ends_with("#!"));
            let after = code[start + "allow".len()..].trim_start();
            hash_before && after.starts_with('(')
        })
        .count()
}

fn doc_groups(comments: &[Comment]) -> Vec<Vec<&Comment>> {
    let mut groups: Vec<Vec<&Comment>> = Vec::new();
    for comment in comments.iter().filter(|c| matches!(c.kind, CommentKind::OuterDoc | CommentKind::InnerDoc)) {
        match groups.last_mut() {
            Some(group)
                if group[0].kind == comment.kind && group.last().is_some_and(|last| last.last_line + 1 == comment.first_line) =>
            {
                group.push(comment)
            }
            _ => groups.push(vec![comment]),
        }
    }
    groups
}

fn qualifies_for_rationale(code: &str, group: &[&Comment]) -> bool {
    let first = group[0];
    let (start, end) = match first.kind {
        CommentKind::InnerDoc => rust_scan::inner_item_extent(code, first.start),
        _ => rust_scan::outer_item_extent(code, group.last().map_or(first.end, |last| last.end)),
    };
    let item = &code[start..end.min(code.len())];
    rust_scan::count_words(item, "unsafe") > 0 || rust_scan::contains_asm_macro(item)
}

pub fn analyze_file(path: &str, source: &str, rationale_exists: &dyn Fn(&str) -> bool) -> FileReport {
    let scanned = rust_scan::scan(source);
    let mut report = FileReport::default();
    let assembly = rust_scan::asm_invocation_ranges(&scanned.code);
    for comment in &scanned.comments {
        let inside_assembly = assembly.iter().any(|&(start, end)| comment.start >= start && comment.start < end);
        if !inside_assembly && matches!(comment.kind, CommentKind::Plain | CommentKind::Block) {
            report.offending_comment_lines += comment.line_count();
        }
    }
    for group in doc_groups(&scanned.comments) {
        let lines: usize = group.iter().map(|c| c.line_count()).sum();
        let text: String = group.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n");
        let links = rationale_links(&text);
        let qualifies = qualifies_for_rationale(&scanned.code, &group);
        let missing: Vec<&String> = links.iter().filter(|link| !rationale_exists(link)).collect();
        let valid = qualifies && !links.is_empty() && missing.is_empty() && lines <= MAX_RATIONALE_COMMENT_LINES;
        if qualifies {
            report.rationale_links.extend(links.iter().cloned());
        }
        if !valid {
            report.offending_comment_lines += lines;
        }
        for link in missing {
            report
                .link_problems
                .push(format!("{path}:{}: doc comment links docs/rationale/{link} which does not exist", group[0].first_line));
        }
    }
    report.allow_attributes = allow_attribute_count(&scanned.code);
    report.unsafe_tokens = rust_scan::count_words(&scanned.code, "unsafe");
    if HAZARD_ROOTS.iter().any(|root| path.starts_with(root)) {
        for token in HAZARD_TOKENS {
            if rust_scan::count_words(&scanned.code, token) > 0 {
                report.hazards.push(format!("{path}: {token} is a 32-bit hazard"));
            }
        }
    }
    report
}

pub fn crate_directory<'a>(path: &str, crate_directories: &'a [String]) -> &'a str {
    crate_directories
        .iter()
        .filter(|directory| directory.is_empty() || path.starts_with(&format!("{directory}/")))
        .max_by_key(|directory| directory.len())
        .map_or("", String::as_str)
}

pub fn parse_baseline(text: &str) -> Result<Baseline, String> {
    let mut baseline = Baseline::new();
    for (index, line) in text.lines().enumerate() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let [gate, key, count] = parts[..] else {
            return Err(format!("baseline line {} must be `<gate> <key> <count>`", index + 1));
        };
        if !GATES.contains(&gate) {
            return Err(format!("baseline line {}: unknown gate {gate}", index + 1));
        }
        let count: usize = count.parse().map_err(|_| format!("baseline line {}: bad count", index + 1))?;
        baseline.insert((gate.to_string(), key.to_string()), count);
    }
    Ok(baseline)
}

pub fn render_baseline(baseline: &Baseline) -> String {
    let mut ordered: Vec<_> = baseline.iter().collect();
    ordered.sort_by(|a, b| (GATES.iter().position(|g| *g == a.0.0), &a.0.1).cmp(&(GATES.iter().position(|g| *g == b.0.0), &b.0.1)));
    ordered
        .into_iter()
        .map(|((gate, key), count)| format!("{gate} {key} {count}\n"))
        .collect()
}

pub fn increases(baseline: &Baseline, actual: &Baseline) -> Vec<(String, String, usize, usize)> {
    actual
        .iter()
        .filter_map(|((gate, key), &count)| {
            let allowed = baseline.get(&(gate.clone(), key.clone())).copied().unwrap_or(0);
            (count > allowed).then(|| (gate.clone(), key.clone(), allowed, count))
        })
        .collect()
}

pub fn compare_baseline(baseline: &Baseline, actual: &Baseline) -> Vec<String> {
    let mut violations: Vec<String> = increases(baseline, actual)
        .into_iter()
        .map(|(gate, key, allowed, count)| format!("{gate}: {key} has {count}, baseline allows {allowed}"))
        .collect();
    for ((gate, key), &allowed) in baseline {
        let count = actual.get(&(gate.clone(), key.clone())).copied().unwrap_or(0);
        if count < allowed {
            violations.push(format!(
                "{gate}: {key} dropped from {allowed} to {count}; ratchet it down with `cargo xtask check-style --update-baseline`"
            ));
        }
    }
    violations
}

pub fn actual_baseline(reports: &BTreeMap<String, FileReport>, crate_directories: &[String]) -> Baseline {
    let mut actual = Baseline::new();
    for (path, report) in reports {
        if report.offending_comment_lines > 0 {
            actual.insert(("comments".into(), path.clone()), report.offending_comment_lines);
        }
        if report.allow_attributes > 0 {
            actual.insert(("allow".into(), path.clone()), report.allow_attributes);
        }
        if report.unsafe_tokens > 0 {
            let directory = crate_directory(path, crate_directories);
            let key = if directory.is_empty() { ".".to_string() } else { directory.to_string() };
            *actual.entry(("unsafe".into(), key)).or_insert(0) += report.unsafe_tokens;
        }
    }
    actual
}

pub fn orphan_rationale_files(existing: &[String], reports: &BTreeMap<String, FileReport>) -> Vec<String> {
    let linked: BTreeSet<&String> = reports.values().flat_map(|report| report.rationale_links.iter()).collect();
    existing
        .iter()
        .filter(|name| !linked.contains(name))
        .map(|name| format!("docs/rationale/{name} is not linked from any doc comment"))
        .collect()
}

fn git_lines(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|err| format!("cannot run git: {err}"))?;
    if !output.status.success() {
        return Err(format!("git {args:?} failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).lines().map(str::to_string).collect())
}

fn rationale_files(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root.join("docs").join("rationale")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|name| name.ends_with(".md") && name != "README.md")
        .collect();
    names.sort();
    names
}

struct Survey {
    reports: BTreeMap<String, FileReport>,
    crate_directories: Vec<String>,
    rationale: Vec<String>,
}

fn survey(root: &Path) -> Result<Survey, String> {
    let rationale = rationale_files(root);
    let rust_files = git_lines(root, &["ls-files", "*.rs"])?;
    let crate_directories: Vec<String> = git_lines(root, &["ls-files", "Cargo.toml", "**/Cargo.toml"])?
        .into_iter()
        .map(|manifest| manifest.strip_suffix("Cargo.toml").unwrap_or("").trim_end_matches('/').to_string())
        .collect();
    let mut reports = BTreeMap::new();
    for path in rust_files {
        let Ok(source) = std::fs::read_to_string(root.join(&path)) else {
            continue;
        };
        let report = analyze_file(&path, &source, &|link| rationale.iter().any(|name| name == link));
        reports.insert(path, report);
    }
    Ok(Survey { reports, crate_directories, rationale })
}

fn read_baseline(root: &Path) -> Result<Option<Baseline>, String> {
    let path = root.join(BASELINE_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    parse_baseline(&text).map(Some)
}

pub fn run(root: &Path) -> Result<Vec<String>, String> {
    let survey = survey(root)?;
    let baseline = read_baseline(root)?.ok_or_else(|| format!("{BASELINE_PATH} is missing; create it with --update-baseline"))?;
    let actual = actual_baseline(&survey.reports, &survey.crate_directories);
    let mut violations = compare_baseline(&baseline, &actual);
    for report in survey.reports.values() {
        violations.extend(report.hazards.iter().cloned());
        violations.extend(report.link_problems.iter().cloned());
    }
    violations.extend(orphan_rationale_files(&survey.rationale, &survey.reports));
    violations.extend(records::validate_directories(&root.join("docs"))?);
    violations.extend(records::validate_architecture_index(root)?);
    Ok(violations)
}

fn today(root: &Path) -> Result<String, String> {
    let output = Command::new("date")
        .arg("+%F")
        .current_dir(root)
        .output()
        .map_err(|err| format!("cannot run date: {err}"))?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn unused_decision_path(directory: &Path, stem: &str) -> std::path::PathBuf {
    let mut candidate = directory.join(format!("{stem}.md"));
    let mut attempt = 2;
    while candidate.exists() {
        candidate = directory.join(format!("{stem}-{attempt}.md"));
        attempt += 1;
    }
    candidate
}

pub fn update_baseline(root: &Path, allow_increase: Option<&str>) -> Result<Vec<String>, String> {
    let survey = survey(root)?;
    let actual = actual_baseline(&survey.reports, &survey.crate_directories);
    let existing = read_baseline(root)?;
    let mut messages = Vec::new();
    if let Some(existing) = &existing {
        let raised = increases(existing, &actual);
        if !raised.is_empty() {
            let Some(reason) = allow_increase else {
                let listing: Vec<String> = raised
                    .iter()
                    .map(|(gate, key, allowed, count)| format!("{gate}: {key} {allowed} -> {count}"))
                    .collect();
                return Err(format!(
                    "refusing to raise the baseline without --allow-increase \"<reason>\":\n  {}",
                    listing.join("\n  ")
                ));
            };
            let date = today(root)?;
            let directory = root.join("docs").join("decisions");
            for gate in GATES {
                let in_gate: Vec<String> = raised
                    .iter()
                    .filter(|(g, ..)| g == gate)
                    .map(|(_, key, allowed, count)| format!("- {key}: {allowed} -> {count}"))
                    .collect();
                if in_gate.is_empty() {
                    continue;
                }
                let path = unused_decision_path(&directory, &format!("gate-baseline-{gate}-raised-{date}"));
                let body = format!(
                    "---\nstatus: decided\ndate: {date}\nrevisit-when: the raised counts can be driven back down\n---\n\n## Question\nWhy was the `{gate}` baseline raised?\n\n## Chosen\n{reason}\n\n{}\n\n## Alternatives\nFixing the cause so the baseline stays put.\n\n## Consequences\nThe `{gate}` ratchet now allows the counts above.\n",
                    in_gate.join("\n")
                );
                std::fs::write(&path, body).map_err(|err| format!("{}: {err}", path.display()))?;
                messages.push(format!("recorded {}", path.display()));
            }
        }
    }
    std::fs::write(root.join(BASELINE_PATH), render_baseline(&actual)).map_err(|err| format!("{BASELINE_PATH}: {err}"))?;
    messages.push(format!("wrote {BASELINE_PATH} with {} entries", actual.len()));
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exists(name: &str) -> bool {
        name == "context-switch.md"
    }

    fn analyze(source: &str) -> FileReport {
        analyze_file("kernel/src/x.rs", source, &exists)
    }

    const LINKED: &str = "/// See docs/rationale/context-switch.md for the invariants.\nunsafe fn swap() {}\n";

    #[test]
    fn plain_comment_is_offending() {
        assert_eq!(analyze("// foo\nfn a() {}\n").offending_comment_lines, 1);
        assert_eq!(analyze("/* a\nb */\nfn a() {}\n").offending_comment_lines, 2);
    }

    #[test]
    fn plain_comments_inside_assembly_invocations_are_exempt() {
        let source = "core::arch::global_asm!(\n    // register use\n    \"nop\", // tail\n);\n// outside\n";
        assert_eq!(analyze(source).offending_comment_lines, 1);
    }

    #[test]
    fn doc_comment_on_safe_item_is_offending() {
        assert_eq!(analyze("/// hello\nfn a() {}\n").offending_comment_lines, 1);
    }

    #[test]
    fn unlinked_doc_comment_on_unsafe_item_is_offending() {
        assert_eq!(analyze("/// hello\nunsafe fn a() {}\n").offending_comment_lines, 1);
    }

    #[test]
    fn linked_doc_comment_on_unsafe_item_passes() {
        let report = analyze(LINKED);
        assert_eq!(report.offending_comment_lines, 0);
        assert!(report.rationale_links.contains("context-switch.md"));
    }

    #[test]
    fn doc_comment_on_item_containing_unsafe_or_asm_passes() {
        let body = "/// docs/rationale/context-switch.md\nfn a() { unsafe { x() } }\n";
        assert_eq!(analyze(body).offending_comment_lines, 0);
        let asm = "/// docs/rationale/context-switch.md\nfn a() { asm!(\"nop\") }\n";
        assert_eq!(analyze(asm).offending_comment_lines, 0);
    }

    #[test]
    fn attributes_between_comment_and_item_are_skipped() {
        let source = "/// docs/rationale/context-switch.md\n#[inline(never)]\nunsafe fn a() {}\n";
        assert_eq!(analyze(source).offending_comment_lines, 0);
    }

    #[test]
    fn link_to_missing_file_is_a_problem_and_offending() {
        let report = analyze("/// docs/rationale/ghost.md\nunsafe fn a() {}\n");
        assert_eq!(report.offending_comment_lines, 1);
        assert_eq!(report.link_problems.len(), 1);
    }

    #[test]
    fn long_rationale_comment_is_offending() {
        let mut source = String::new();
        for _ in 0..6 {
            source.push_str("/// docs/rationale/context-switch.md\n");
        }
        source.push_str("unsafe fn a() {}\n");
        assert_eq!(analyze(&source).offending_comment_lines, 0);
        let seven = format!("/// more\n{source}");
        assert_eq!(analyze(&seven).offending_comment_lines, 7);
    }

    #[test]
    fn inner_doc_at_file_top_attaches_to_the_file() {
        let source = "//! docs/rationale/context-switch.md\nfn a() { unsafe {} }\n";
        assert_eq!(analyze(source).offending_comment_lines, 0);
        let safe = "//! docs/rationale/context-switch.md\nfn a() {}\n";
        assert_eq!(analyze(safe).offending_comment_lines, 1);
    }

    #[test]
    fn four_slashes_is_a_plain_comment() {
        assert_eq!(analyze("//// x\nunsafe fn a() {}\n").offending_comment_lines, 1);
    }

    #[test]
    fn rationale_links_need_a_valid_slug_and_extension() {
        assert_eq!(rationale_links("docs/rationale/a-b1_x.md#frag and docs/rationale/x.txt docs/rationale/.md"), vec!["a-b1_x.md"]);
    }

    #[test]
    fn allow_attributes_are_counted_not_expect() {
        assert_eq!(allow_attribute_count("#[allow(dead_code)] #![allow (x)] #[expect(dead_code)] fn allow() {}"), 2);
        assert_eq!(allow_attribute_count("let allow = 1; allow(x);"), 0);
    }

    #[test]
    fn unsafe_tokens_ignore_strings_and_comments() {
        let report = analyze("fn a() { unsafe {} let s = \"unsafe\"; } // unsafe\n");
        assert_eq!(report.unsafe_tokens, 1);
    }

    #[test]
    fn thirty_two_bit_hazards_are_flagged_only_in_guarded_roots() {
        let source = "static C: AtomicU64 = x; fn f(a: f32) {}\n";
        assert_eq!(analyze_file("kernel/src/a.rs", source, &exists).hazards.len(), 2);
        assert!(analyze_file("xtask/src/a.rs", source, &exists).hazards.is_empty());
        assert!(analyze_file("kernel/src/a.rs", "// f64\nlet s = \"f64\";", &exists).hazards.is_empty());
    }

    #[test]
    fn crate_directory_picks_the_longest_prefix() {
        let dirs = vec!["".to_string(), "arch".to_string(), "arch/x86_32".to_string()];
        assert_eq!(crate_directory("arch/x86_32/src/a.rs", &dirs), "arch/x86_32");
        assert_eq!(crate_directory("build.rs", &dirs), "");
    }

    fn baseline(entries: &[(&str, &str, usize)]) -> Baseline {
        entries.iter().map(|(g, k, c)| ((g.to_string(), k.to_string()), *c)).collect()
    }

    #[test]
    fn baseline_round_trips() {
        let original = baseline(&[("comments", "a.rs", 3), ("unsafe", "kernel", 10), ("allow", "b.rs", 1)]);
        assert_eq!(parse_baseline(&render_baseline(&original)).unwrap(), original);
        assert!(parse_baseline("comments a.rs").is_err());
        assert!(parse_baseline("bogus a.rs 1").is_err());
    }

    #[test]
    fn baseline_comparison_is_a_two_way_ratchet() {
        let base = baseline(&[("comments", "a.rs", 3)]);
        assert!(compare_baseline(&base, &base).is_empty());
        let raised = baseline(&[("comments", "a.rs", 4)]);
        assert!(compare_baseline(&base, &raised)[0].contains("baseline allows 3"));
        let fresh = baseline(&[("comments", "a.rs", 3), ("comments", "b.rs", 1)]);
        assert_eq!(compare_baseline(&base, &fresh).len(), 1);
        let dropped = baseline(&[("comments", "a.rs", 2)]);
        assert!(compare_baseline(&base, &dropped)[0].contains("dropped"));
        assert!(compare_baseline(&base, &Baseline::new())[0].contains("dropped"));
    }

    #[test]
    fn orphan_rationale_files_are_reported() {
        let mut reports = BTreeMap::new();
        reports.insert("a.rs".to_string(), analyze(LINKED));
        let existing = vec!["context-switch.md".to_string(), "lonely.md".to_string()];
        let orphans = orphan_rationale_files(&existing, &reports);
        assert_eq!(orphans, vec!["docs/rationale/lonely.md is not linked from any doc comment".to_string()]);
    }

    #[test]
    fn unsafe_counts_roll_up_per_crate() {
        let mut reports = BTreeMap::new();
        reports.insert("kernel/src/a.rs".to_string(), analyze("fn a() { unsafe {} unsafe {} }"));
        reports.insert("kernel/src/b.rs".to_string(), analyze("fn a() { unsafe {} }"));
        let dirs = vec!["kernel".to_string()];
        let actual = actual_baseline(&reports, &dirs);
        assert_eq!(actual.get(&("unsafe".into(), "kernel".into())), Some(&3));
    }
}
