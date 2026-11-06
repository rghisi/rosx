use alloc::string::String;
use alloc::vec::Vec;
use vfs_protocol::wire;

pub const ROM_FILE_NAME: &str = "init.lst";

#[derive(Debug, PartialEq)]
pub struct Manifest {
    pub services: Vec<String>,
    pub handoff: Option<String>,
    pub fallback: Option<String>,
}

enum Verb {
    Service(String),
    Handoff(String),
    Fallback(String),
}

pub fn parse(bytes: &[u8]) -> (Manifest, usize) {
    let mut services = Vec::new();
    let mut handoff: Option<String> = None;
    let mut fallback: Option<String> = None;
    let mut skipped = 0usize;
    for line in bytes.split(|byte| *byte == b'\n') {
        let line = match core::str::from_utf8(line) {
            Ok(line) => line.trim_end_matches('\r').trim(),
            Err(_) => {
                if !line.is_empty() {
                    skipped += 1;
                }
                continue;
            }
        };
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_line(line) {
            Some(Verb::Service(name)) => services.push(name),
            Some(Verb::Handoff(path)) => {
                if handoff.is_some() {
                    skipped += 1;
                } else {
                    handoff = Some(path);
                }
            }
            Some(Verb::Fallback(name)) => {
                if fallback.is_some() {
                    skipped += 1;
                } else {
                    fallback = Some(name);
                }
            }
            None => skipped += 1,
        }
    }
    (Manifest { services, handoff, fallback }, skipped)
}

fn parse_line(line: &str) -> Option<Verb> {
    let mut fields = line.split_whitespace();
    let verb = fields.next()?;
    let target = fields.next()?;
    if fields.next().is_some() {
        return None;
    }
    match verb {
        "service" if wire::valid_name(target) => Some(Verb::Service(String::from(target))),
        "handoff" if wire::valid_path(target) => Some(Verb::Handoff(String::from(target))),
        "fallback" if wire::valid_name(target) => Some(Verb::Fallback(String::from(target))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_of(text: &str) -> Manifest {
        let (manifest, skipped) = parse(text.as_bytes());
        assert_eq!(skipped, 0, "clean manifest expected, text: {text:?}");
        manifest
    }

    #[test]
    fn default_manifest_parses() {
        let manifest = manifest_of(
            "service randomd\nservice echo\nservice bulk\nservice pong\nservice vfsd\nservice inputd\nservice ps2d\nhandoff hdd/shell\nfallback shell\n",
        );
        assert_eq!(
            manifest.services,
            ["randomd", "echo", "bulk", "pong", "vfsd", "inputd", "ps2d"]
        );
        assert_eq!(manifest.handoff.as_deref(), Some("hdd/shell"));
        assert_eq!(manifest.fallback.as_deref(), Some("shell"));
    }

    #[test]
    fn comments_and_blank_lines_are_skipped_not_counted() {
        let (manifest, skipped) =
            parse(b"# boot manifest\n\n  \n\t\nservice vfsd\n");
        assert_eq!(manifest.services, ["vfsd"]);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn carriage_returns_are_tolerated() {
        let (manifest, skipped) = parse(b"# manifest\r\nservice vfsd\r\nhandoff hdd/shell\r\n");
        assert_eq!(manifest.services, ["vfsd"]);
        assert_eq!(manifest.handoff.as_deref(), Some("hdd/shell"));
        assert_eq!(skipped, 0);
    }

    #[test]
    fn last_line_needs_no_trailing_newline() {
        let (manifest, skipped) = parse(b"service vfsd");
        assert_eq!(manifest.services, ["vfsd"]);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn tabs_and_padded_whitespace_separate_fields() {
        let (manifest, skipped) = parse(b"   service\t  vfsd  \n\tfallback\tshell\t\n");
        assert_eq!(manifest.services, ["vfsd"]);
        assert_eq!(manifest.fallback.as_deref(), Some("shell"));
        assert_eq!(skipped, 0);
    }

    #[test]
    fn services_keep_their_file_order() {
        let manifest = manifest_of("service pong\nservice echo\nservice randomd\n");
        assert_eq!(manifest.services, ["pong", "echo", "randomd"]);
    }

    #[test]
    fn empty_input_yields_an_empty_manifest() {
        assert_eq!(parse(b""), (Manifest { services: Vec::new(), handoff: None, fallback: None }, 0));
    }

    #[test]
    fn headless_manifest_is_complete() {
        let manifest = manifest_of("service vfsd\nservice inputd\n");
        assert_eq!(manifest.services, ["vfsd", "inputd"]);
        assert_eq!(manifest.handoff, None);
        assert_eq!(manifest.fallback, None);
    }

    #[test]
    fn handoff_and_fallback_alone_are_legal() {
        let manifest = manifest_of("handoff hdd/shell\nfallback shell\n");
        assert!(manifest.services.is_empty());
        assert_eq!(manifest.handoff.as_deref(), Some("hdd/shell"));
        assert_eq!(manifest.fallback.as_deref(), Some("shell"));
    }

    #[test]
    fn unknown_verb_is_skipped() {
        let (manifest, skipped) = parse(b"daemon vfsd\nservice echo\n");
        assert_eq!(manifest.services, ["echo"]);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn missing_target_is_skipped() {
        assert_eq!(parse(b"service\n").1, 1);
        assert_eq!(parse(b"handoff\n").1, 1);
        assert_eq!(parse(b"fallback\n").1, 1);
    }

    #[test]
    fn too_many_fields_is_skipped() {
        let (manifest, skipped) = parse(b"service vfsd extra\nservice echo\n");
        assert_eq!(manifest.services, ["echo"]);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn duplicate_handoff_is_skipped_and_the_first_wins() {
        let (manifest, skipped) =
            parse(b"handoff hdd/shell\nhandoff other/thing\nservice vfsd\n");
        assert_eq!(manifest.handoff.as_deref(), Some("hdd/shell"));
        assert_eq!(skipped, 1);
    }

    #[test]
    fn duplicate_fallback_is_skipped_and_the_first_wins() {
        let (manifest, skipped) = parse(b"fallback shell\nfallback dummy\n");
        assert_eq!(manifest.fallback.as_deref(), Some("shell"));
        assert_eq!(skipped, 1);
    }

    #[test]
    fn duplicate_service_lines_are_legal() {
        let (manifest, skipped) = parse(b"service echo\nservice echo\n");
        assert_eq!(manifest.services, ["echo", "echo"]);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn targets_rejected_by_the_wire_treaty_are_skipped() {
        assert_eq!(parse(b"service bad/name\n").1, 1);
        assert_eq!(parse(b"service .\n").1, 1);
        assert_eq!(parse(b"service ..\n").1, 1);
        assert_eq!(parse(b"fallback bad/name\n").1, 1);
    }

    #[test]
    fn handoff_targets_that_are_not_valid_paths_are_skipped() {
        assert_eq!(parse(b"handoff /\n").1, 1);
        assert_eq!(parse(b"handoff hdd//shell\n").1, 1);
        assert_eq!(parse(b"handoff hdd/../shell\n").1, 1);
        assert_eq!(parse(b"handoff .\n").1, 1);
    }

    #[test]
    fn malformed_lines_cost_lines_not_the_whole_manifest() {
        let (manifest, skipped) = parse(
            b"service vfsd\nbogus line here\nhandoff\nservice inputd\nfallback shell\n",
        );
        assert_eq!(manifest.services, ["vfsd", "inputd"]);
        assert_eq!(manifest.handoff, None);
        assert_eq!(manifest.fallback.as_deref(), Some("shell"));
        assert_eq!(skipped, 2);
    }

    #[test]
    fn non_utf8_line_is_skipped_but_siblings_survive() {
        let (manifest, skipped) = parse(b"service vfsd\n\xff\xfe junk\nservice echo\n");
        assert_eq!(manifest.services, ["vfsd", "echo"]);
        assert_eq!(skipped, 1);
    }
}
