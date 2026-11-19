use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use crate::json::{self, Json};

const FEATURE_FLOOR: &str = "-mmx,-sse,-sse2,-cmov,-fxsr,-cx8";
const PINNED_TARGET_SPECS: [&str; 2] = ["arch/x86_32/rosx-i486.json", "arch/x86_32/rosx-i486-user.json"];

pub type Dependencies = BTreeMap<String, BTreeSet<String>>;

pub fn parse_rules(text: &str) -> Result<Dependencies, String> {
    let mut rules = Dependencies::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (name, allowed) = line
            .split_once(':')
            .ok_or_else(|| format!("arch-rules line {} has no ':'", index + 1))?;
        let allowed = allowed.split_whitespace().map(str::to_string).collect();
        if rules.insert(name.trim().to_string(), allowed).is_some() {
            return Err(format!("arch-rules lists {} twice", name.trim()));
        }
    }
    Ok(rules)
}

pub fn workspace_dependencies(metadata: &Json) -> Result<Dependencies, String> {
    let mut found = Dependencies::new();
    for package in metadata.get("packages").map(Json::as_array).unwrap_or(&[]) {
        let name = package
            .get("name")
            .and_then(Json::as_str)
            .ok_or_else(|| "package without a name".to_string())?;
        let dependencies = package
            .get("dependencies")
            .map(Json::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|dependency| dependency.get("name").and_then(Json::as_str))
            .map(str::to_string)
            .collect();
        found.insert(name.to_string(), dependencies);
    }
    Ok(found)
}

pub fn compare(rules: &Dependencies, found: &Dependencies) -> Vec<String> {
    let mut violations = Vec::new();
    for (name, dependencies) in found {
        match rules.get(name) {
            None => violations.push(format!("{name}: workspace crate has no rule in xtask/arch-rules.txt")),
            Some(allowed) => {
                for dependency in dependencies.difference(allowed) {
                    violations.push(format!("{name}: depends on {dependency}, which its rule does not allow"));
                }
            }
        }
    }
    for name in rules.keys().filter(|name| !found.contains_key(*name)) {
        violations.push(format!("{name}: rule names a crate that is not in the workspace"));
    }
    violations
}

pub fn feature_floor_violations(target_name: &str, spec_text: &str) -> Vec<String> {
    let declared = json::parse(spec_text)
        .ok()
        .and_then(|spec| spec.get("features").and_then(Json::as_str).map(str::to_string));
    let Some(features) = declared else {
        return vec![format!("{target_name}: no features string")];
    };
    if features == FEATURE_FLOOR {
        Vec::new()
    } else {
        vec![format!("{target_name}: features {features:?} differ from the pinned 486 floor {FEATURE_FLOOR:?}")]
    }
}

pub fn run(root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .map_err(|err| format!("cannot run cargo metadata: {err}"))?;
    if !output.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    let metadata = json::parse(&String::from_utf8_lossy(&output.stdout))?;
    let found = workspace_dependencies(&metadata)?;
    let rules_path = root.join("xtask").join("arch-rules.txt");
    let rules = parse_rules(
        &std::fs::read_to_string(&rules_path).map_err(|err| format!("{}: {err}", rules_path.display()))?,
    )?;
    let mut violations = compare(&rules, &found);
    for relative in PINNED_TARGET_SPECS {
        let path = root.join(relative);
        let text = std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        violations.extend(feature_floor_violations(relative, &text));
    }
    Ok(violations)
}

#[cfg(test)]
mod tests {
    use super::*;

    const METADATA: &str = r#"{"packages":[
        {"name":"system","dependencies":[]},
        {"name":"kernel","dependencies":[{"name":"system"}]}
    ]}"#;

    fn found() -> Dependencies {
        workspace_dependencies(&json::parse(METADATA).unwrap()).unwrap()
    }

    #[test]
    fn matching_rules_pass() {
        let rules = parse_rules("system:\nkernel: system\n").unwrap();
        assert!(compare(&rules, &found()).is_empty());
    }

    #[test]
    fn new_dependency_on_system_fails() {
        let rules = parse_rules("system:\nkernel: system\n").unwrap();
        let mut changed = found();
        changed.get_mut("system").unwrap().insert("spin".into());
        let violations = compare(&rules, &changed);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("system: depends on spin"));
    }

    #[test]
    fn unruled_and_stale_crates_fail() {
        let rules = parse_rules("system:\nkernel: system\nghost:\n").unwrap();
        let mut changed = found();
        changed.insert("fresh".into(), BTreeSet::new());
        let violations = compare(&rules, &changed);
        assert!(violations.iter().any(|v| v.starts_with("fresh:")));
        assert!(violations.iter().any(|v| v.starts_with("ghost:")));
    }

    #[test]
    fn duplicate_or_malformed_rules_are_rejected() {
        assert!(parse_rules("a:\na:\n").is_err());
        assert!(parse_rules("no colon\n").is_err());
    }

    #[test]
    fn feature_floor_detects_widening() {
        let pinned = format!(r#"{{"features":"{FEATURE_FLOOR}"}}"#);
        assert!(feature_floor_violations("t", &pinned).is_empty());
        let narrowed = r#"{"features":"-mmx,-sse,-sse2,-cmov,-fxsr"}"#;
        assert_eq!(feature_floor_violations("t", narrowed).len(), 1);
        let widened = format!(r#"{{"features":"{FEATURE_FLOOR},+sse"}}"#);
        assert_eq!(feature_floor_violations("t", &widened).len(), 1);
        assert_eq!(feature_floor_violations("t", "{}").len(), 1);
    }
}
