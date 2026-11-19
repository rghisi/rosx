use std::collections::BTreeMap;
use std::path::Path;

pub const PRIORITIES: [&str; 3] = ["high", "normal", "low"];
const DECISION_STATUSES: [&str; 3] = ["open", "decided", "superseded"];
const DECISION_SECTIONS: [&str; 4] = ["## Question", "## Chosen", "## Alternatives", "## Consequences"];

pub fn is_kebab_markdown_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".md") else {
        return false;
    };
    !stem.is_empty()
        && stem
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

pub fn is_iso_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, &b)| match index {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        })
}

pub fn front_matter(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return Err("front matter must start with ---".into());
    }
    let mut fields = BTreeMap::new();
    for line in lines {
        if line == "---" {
            return Ok(fields);
        }
        let (key, value) = line.split_once(':').ok_or_else(|| format!("malformed front-matter line {line:?}"))?;
        fields.insert(key.trim().to_string(), value.trim().to_string());
    }
    Err("front matter is not closed by ---".into())
}

fn required<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, String> {
    fields
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing front-matter key {key}"))
}

pub fn validate_decision(name: &str, text: &str, existing: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut problems = Vec::new();
    if !is_kebab_markdown_name(name) {
        problems.push(format!("{name}: filename must be kebab-case .md"));
    }
    match front_matter(text) {
        Err(message) => problems.push(format!("{name}: {message}")),
        Ok(fields) => {
            match required(&fields, "status") {
                Ok(status) if DECISION_STATUSES.contains(&status) => {}
                Ok(status) => problems.push(format!("{name}: invalid status {status:?}")),
                Err(message) => problems.push(format!("{name}: {message}")),
            }
            match required(&fields, "date") {
                Ok(date) if is_iso_date(date) => {}
                Ok(date) => problems.push(format!("{name}: date {date:?} is not YYYY-MM-DD")),
                Err(message) => problems.push(format!("{name}: {message}")),
            }
            if let Err(message) = required(&fields, "revisit-when") {
                problems.push(format!("{name}: {message}"));
            }
            if let Some(target) = fields.get("supersedes") {
                if !existing(target) {
                    problems.push(format!("{name}: supersedes {target} which does not exist"));
                }
            }
        }
    }
    for section in DECISION_SECTIONS {
        if !text.lines().any(|line| line == section) {
            problems.push(format!("{name}: missing section {section}"));
        }
    }
    problems
}

pub fn validate_todo(name: &str, text: &str, decision_exists: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut problems = Vec::new();
    if !is_kebab_markdown_name(name) {
        problems.push(format!("{name}: filename must be kebab-case .md"));
    }
    match front_matter(text) {
        Err(message) => problems.push(format!("{name}: {message}")),
        Ok(fields) => {
            match required(&fields, "priority") {
                Ok(priority) if PRIORITIES.contains(&priority) => {}
                Ok(priority) => problems.push(format!("{name}: invalid priority {priority:?}")),
                Err(message) => problems.push(format!("{name}: {message}")),
            }
            match required(&fields, "date") {
                Ok(date) if is_iso_date(date) => {}
                Ok(date) => problems.push(format!("{name}: date {date:?} is not YYYY-MM-DD")),
                Err(message) => problems.push(format!("{name}: {message}")),
            }
            if let Err(message) = required(&fields, "trigger") {
                problems.push(format!("{name}: {message}"));
            }
            if let Some(target) = fields.get("decision") {
                if !decision_exists(target) {
                    problems.push(format!("{name}: decision {target} does not exist"));
                }
            }
        }
    }
    problems
}

fn markdown_files(directory: &Path) -> Result<Vec<(String, String)>, String> {
    let mut files = Vec::new();
    let entries = std::fs::read_dir(directory).map_err(|err| format!("{}: {err}", directory.display()))?;
    for entry in entries {
        let path = entry.map_err(|err| err.to_string())?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if name == "README.md" || name.starts_with('.') {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        files.push((name, text));
    }
    files.sort();
    Ok(files)
}

pub fn validate_directories(docs: &Path) -> Result<Vec<String>, String> {
    let decisions = markdown_files(&docs.join("decisions"))?;
    let todos = markdown_files(&docs.join("todo"))?;
    let decision_names: Vec<&str> = decisions.iter().map(|(name, _)| name.as_str()).collect();
    let decision_exists = |target: &str| decision_names.contains(&target);
    let mut problems = Vec::new();
    for (name, text) in &decisions {
        problems.extend(validate_decision(name, text, &decision_exists));
    }
    for (name, text) in &todos {
        problems.extend(validate_todo(name, text, &decision_exists));
    }
    Ok(problems)
}

pub fn list_open(docs: &Path) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    lines.push("open decisions:".to_string());
    for (name, text) in markdown_files(&docs.join("decisions"))? {
        let Ok(fields) = front_matter(&text) else { continue };
        if fields.get("status").map(String::as_str) == Some("open") {
            lines.push(format!("  {name}  (revisit when: {})", fields.get("revisit-when").map_or("?", String::as_str)));
        }
    }
    let todos = markdown_files(&docs.join("todo"))?;
    for priority in PRIORITIES {
        lines.push(format!("todo, {priority} priority:"));
        for (name, text) in &todos {
            let Ok(fields) = front_matter(text) else { continue };
            if fields.get("priority").map(String::as_str) == Some(priority) {
                lines.push(format!("  {name}  (trigger: {})", fields.get("trigger").map_or("?", String::as_str)));
            }
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_DECISION: &str = "---\nstatus: open\ndate: 2026-11-19\nrevisit-when: never\n---\n\n## Question\n## Chosen\n## Alternatives\n## Consequences\n";

    #[test]
    fn kebab_names() {
        assert!(is_kebab_markdown_name("a-b2.md"));
        assert!(!is_kebab_markdown_name("A-b.md"));
        assert!(!is_kebab_markdown_name("a_b.md"));
        assert!(!is_kebab_markdown_name("a--b.md"));
        assert!(!is_kebab_markdown_name("a.txt"));
        assert!(!is_kebab_markdown_name(".md"));
    }

    #[test]
    fn dates() {
        assert!(is_iso_date("2026-11-19"));
        assert!(!is_iso_date("2026-1-19"));
        assert!(!is_iso_date("20261119xx"));
    }

    #[test]
    fn good_decision_has_no_problems() {
        assert!(validate_decision("a-b.md", GOOD_DECISION, &|_| false).is_empty());
    }

    #[test]
    fn decision_problems_are_each_reported() {
        let bad = "---\nstatus: maybe\ndate: soon\nsupersedes: ghost.md\n---\n";
        let problems = validate_decision("Bad_Name.md", bad, &|_| false);
        assert!(problems.iter().any(|p| p.contains("kebab-case")));
        assert!(problems.iter().any(|p| p.contains("invalid status")));
        assert!(problems.iter().any(|p| p.contains("not YYYY-MM-DD")));
        assert!(problems.iter().any(|p| p.contains("revisit-when")));
        assert!(problems.iter().any(|p| p.contains("supersedes ghost.md")));
        assert!(problems.iter().any(|p| p.contains("## Question")));
    }

    #[test]
    fn supersedes_target_may_exist() {
        let text = GOOD_DECISION.replace("revisit-when: never", "revisit-when: never\nsupersedes: old.md");
        assert!(validate_decision("a.md", &text, &|target| target == "old.md").is_empty());
    }

    #[test]
    fn todo_validation() {
        let good = "---\npriority: low\ndate: 2026-11-19\ntrigger: none\ndecision: d.md\n---\nbody\n";
        assert!(validate_todo("a.md", good, &|t| t == "d.md").is_empty());
        assert_eq!(validate_todo("a.md", good, &|_| false).len(), 1);
        let bad = "---\npriority: urgent\ndate: 2026-11-19\n---\n";
        let problems = validate_todo("a.md", bad, &|_| true);
        assert!(problems.iter().any(|p| p.contains("invalid priority")));
        assert!(problems.iter().any(|p| p.contains("trigger")));
        assert_eq!(validate_todo("a.md", "no front matter", &|_| true).len(), 1);
    }

    #[test]
    fn front_matter_must_close() {
        assert!(front_matter("---\na: b\n").is_err());
        assert_eq!(front_matter("---\na: b\n---\n").unwrap().get("a").map(String::as_str), Some("b"));
    }
}
