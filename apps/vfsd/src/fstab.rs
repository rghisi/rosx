use alloc::string::String;
use alloc::vec::Vec;
use vfs_protocol::wire;

pub const ROM_FILE_NAME: &str = "fstab";
pub const ROOT_FILE_NAME: &str = "root";

#[derive(Debug, PartialEq)]
pub struct FstabEntry {
    pub alias: String,
    pub service: String,
    pub drive: u32,
    pub dir: String,
}

#[derive(Debug, PartialEq)]
pub struct RootTarget {
    pub alias: String,
    pub service: String,
    pub drive: u32,
}

pub fn parse(bytes: &[u8]) -> (Vec<FstabEntry>, usize) {
    let mut entries = Vec::new();
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
            Some(entry) => entries.push(entry),
            None => skipped += 1,
        }
    }
    (entries, skipped)
}

fn parse_line(line: &str) -> Option<FstabEntry> {
    let mut fields = line.split_whitespace();
    let alias = fields.next()?;
    let service = fields.next()?;
    let drive = fields.next()?;
    let dir = fields.next()?;
    if fields.next().is_some() {
        return None;
    }
    if !wire::valid_name(alias) || !wire::valid_name(service) {
        return None;
    }
    let drive = drive.parse::<u32>().ok()?;
    let dir = dir.strip_prefix('/').unwrap_or(dir);
    if !wire::valid_name(dir) {
        return None;
    }
    Some(FstabEntry {
        alias: String::from(alias),
        service: String::from(service),
        drive,
        dir: String::from(dir),
    })
}

pub fn parse_root(bytes: &[u8]) -> (Option<RootTarget>, usize) {
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
        match parse_root_line(line) {
            Some(target) => return (Some(target), skipped),
            None => skipped += 1,
        }
    }
    (None, skipped)
}

fn parse_root_line(line: &str) -> Option<RootTarget> {
    let mut fields = line.split_whitespace();
    let alias = fields.next()?;
    let service = fields.next()?;
    let drive = fields.next()?;
    if fields.next().is_some() {
        return None;
    }
    if !wire::valid_name(alias) || !wire::valid_name(service) {
        return None;
    }
    let drive = drive.parse::<u32>().ok()?;
    Some(RootTarget {
        alias: String::from(alias),
        service: String::from(service),
        drive,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_one(text: &str) -> FstabEntry {
        let (entries, skipped) = parse(text.as_bytes());
        assert_eq!(skipped, 0, "default table must parse cleanly");
        assert_eq!(entries.len(), 1);
        entries.into_iter().next().expect("one entry")
    }

    #[test]
    fn default_convention_line_parses() {
        assert_eq!(
            parse_one("hdd0 ATA 1 /hdd\n"),
            FstabEntry {
                alias: String::from("hdd0"),
                service: String::from("ATA"),
                drive: 1,
                dir: String::from("hdd"),
            }
        );
    }

    #[test]
    fn bare_dir_without_leading_slash_parses_the_same() {
        assert_eq!(parse_one("hdd0 ATA 1 hdd"), parse_one("hdd0 ATA 1 /hdd"));
    }

    #[test]
    fn comments_and_blank_lines_are_skipped_not_counted() {
        let (entries, skipped) = parse(b"# boot mount table\n\n  \n\t\nhdd0 ATA 1 /hdd\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn carriage_returns_are_tolerated() {
        let (entries, skipped) = parse(b"# table\r\nhdd0 ATA 1 /hdd\r\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn last_line_needs_no_trailing_newline() {
        let (entries, skipped) = parse(b"hdd0 ATA 1 /hdd");
        assert_eq!(entries.len(), 1);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn tabs_and_padded_whitespace_separate_fields() {
        let (entries, _) = parse(b"  hdd0\tATA   1\t/hdd  \n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].alias, "hdd0");
        assert_eq!(entries[0].drive, 1);
    }

    #[test]
    fn multiple_entries_keep_their_file_order() {
        let (entries, skipped) = parse(b"a0 ATA 1 /first\nb0 ATA 2 /second\n");
        assert_eq!(skipped, 0);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].alias, "a0");
        assert_eq!(entries[0].dir, "first");
        assert_eq!(entries[1].alias, "b0");
        assert_eq!(entries[1].drive, 2);
    }

    #[test]
    fn drive_zero_is_a_legal_drive_number() {
        let (entries, _) = parse(b"d0 ATA 0 /d0\n");
        assert_eq!(entries[0].drive, 0);
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert_eq!(parse(b""), (Vec::new(), 0));
    }

    #[test]
    fn too_few_fields_is_skipped() {
        assert_eq!(parse(b"hdd0 ATA 1\n").1, 1);
        assert_eq!(parse(b"hdd0\n").1, 1);
    }

    #[test]
    fn too_many_fields_is_skipped() {
        assert_eq!(parse(b"hdd0 ATA 1 /hdd extra\n").1, 1);
    }

    #[test]
    fn non_numeric_and_overflowing_drive_are_skipped() {
        assert_eq!(parse(b"hdd0 ATA one /hdd\n").1, 1);
        assert_eq!(parse(b"hdd0 ATA 4294967296 /hdd\n").1, 1);
        assert_eq!(parse(b"hdd0 ATA -1 /hdd\n").1, 1);
    }

    #[test]
    fn names_rejected_by_the_wire_treaty_are_skipped() {
        assert_eq!(parse(b"bad/alias ATA 1 /hdd\n").1, 1);
        assert_eq!(parse(b". ATA 1 /hdd\n").1, 1);
        assert_eq!(parse(b"srv..bad. ATA 1 /hdd\n").1, 0, "dotted names are valid wire names");
        assert_eq!(parse(b"hdd0 AT A 1 /hdd x\n").1, 1);
    }

    #[test]
    fn dirs_that_are_not_one_valid_name_are_skipped() {
        assert_eq!(parse(b"hdd0 ATA 1 /\n").1, 1);
        assert_eq!(parse(b"hdd0 ATA 1 /nested/deep\n").1, 1);
        assert_eq!(parse(b"hdd0 ATA 1 .\n").1, 1);
        assert_eq!(parse(b"hdd0 ATA 1 ..\n").1, 1);
    }

    #[test]
    fn malformed_lines_cost_lines_not_the_whole_table() {
        let (entries, skipped) = parse(b"good0 ATA 1 /good\nbroken line\nATA 2\nok0 ATA 2 /ok\n");
        assert_eq!(skipped, 2);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].alias, "good0");
        assert_eq!(entries[1].alias, "ok0");
    }

    #[test]
    fn non_utf8_line_is_skipped_but_siblings_survive() {
        let (entries, skipped) = parse(b"good0 ATA 1 /good\n\xff\xfe junk\nok0 ATA 2 /ok\n");
        assert_eq!(skipped, 1);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn comments_are_line_start_only_never_inline() {
        let (entries, skipped) = parse(b"hdd0 ATA 1 /hdd # trailing words\n");
        assert_eq!(entries.len(), 0, "a fifth field makes the line malformed");
        assert_eq!(skipped, 1);
        let (entries, skipped) = parse(b"hdd0 # inline\n");
        assert_eq!(entries.len(), 0);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn root_default_convention_line_parses() {
        let (target, skipped) = parse_root(b"# boot root: alias service drive\nroot0 ATA 0\n");
        assert_eq!(skipped, 0);
        assert_eq!(
            target,
            Some(RootTarget {
                alias: String::from("root0"),
                service: String::from("ATA"),
                drive: 0,
            })
        );
    }

    #[test]
    fn root_first_valid_line_wins_and_later_lines_are_ignored() {
        let (target, skipped) = parse_root(b"a0 ATA 0\nb1 ATA 1\n");
        assert_eq!(target.map(|t| t.alias), Some(String::from("a0")));
        assert_eq!(skipped, 0);
    }

    #[test]
    fn root_malformed_lines_cost_lines_not_the_record() {
        let (target, skipped) = parse_root(b"oops ATA x 0\nroot0 ATA 2\n");
        assert_eq!(skipped, 1);
        assert_eq!(target.map(|t| t.drive), Some(2));
    }

    #[test]
    fn root_rejects_wrong_field_counts_and_bad_names() {
        assert_eq!(parse_root(b"root0 ATA\n").0, None);
        assert_eq!(parse_root(b"root0 ATA 0 extra\n").0, None);
        assert_eq!(parse_root(b"bad/alias ATA 0\n").0, None);
        assert_eq!(parse_root(b"root0 ATA one\n").0, None);
        assert_eq!(parse_root(b"root0 ATA -1\n").0, None);
    }

    #[test]
    fn root_empty_and_comment_only_input_yield_nothing() {
        assert_eq!(parse_root(b""), (None, 0));
        assert_eq!(parse_root(b"# nothing here\n\n"), (None, 0));
    }
}
