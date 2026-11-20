#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Directory {
    Bin,
    Games,
    Tests,
    Bench,
    Stress,
    Srv,
    Boot,
}

impl Directory {
    pub fn name(self) -> &'static str {
        match self {
            Self::Bin => "bin",
            Self::Games => "games",
            Self::Tests => "tests",
            Self::Bench => "bench",
            Self::Stress => "stress",
            Self::Srv => "srv",
            Self::Boot => "boot",
        }
    }
}

pub const ALL_DIRECTORIES: [Directory; 7] = [
    Directory::Bin,
    Directory::Games,
    Directory::Tests,
    Directory::Bench,
    Directory::Stress,
    Directory::Srv,
    Directory::Boot,
];

pub const SEARCH_PATH: [Directory; 5] = [
    Directory::Bin,
    Directory::Tests,
    Directory::Games,
    Directory::Bench,
    Directory::Stress,
];

pub const APP_DIRECTORIES: [(&str, Directory); 43] = [
    ("pi", Directory::Stress),
    ("randomd", Directory::Srv),
    ("echo", Directory::Srv),
    ("bulk", Directory::Tests),
    ("pong", Directory::Srv),
    ("ping", Directory::Tests),
    ("mem", Directory::Bin),
    ("ps", Directory::Bin),
    ("vfsd", Directory::Srv),
    ("vfs", Directory::Bin),
    ("snake", Directory::Games),
    ("tetris", Directory::Games),
    ("conway", Directory::Games),
    ("threads", Directory::Tests),
    ("waitset", Directory::Tests),
    ("inputd", Directory::Srv),
    ("ps2d", Directory::Srv),
    ("stress", Directory::Stress),
    ("bench", Directory::Bench),
    ("fsbench", Directory::Bench),
    ("atad", Directory::Srv),
    ("blockd", Directory::Srv),
    ("ata", Directory::Bin),
    ("format", Directory::Bin),
    ("mount", Directory::Bin),
    ("attach", Directory::Bin),
    ("rm", Directory::Bin),
    ("cp", Directory::Bin),
    ("mkdir", Directory::Bin),
    ("rmdir", Directory::Bin),
    ("pcid", Directory::Srv),
    ("ne2kd", Directory::Srv),
    ("pci", Directory::Bin),
    ("ne2k", Directory::Bin),
    ("arp", Directory::Bin),
    ("ipd", Directory::Srv),
    ("ip", Directory::Bin),
    ("leaky", Directory::Tests),
    ("ted", Directory::Bin),
    ("init", Directory::Boot),
    ("shell", Directory::Bin),
    ("exiter", Directory::Tests),
    ("spinner", Directory::Tests),
];

pub fn directory_of(app: &str) -> Option<Directory> {
    APP_DIRECTORIES
        .iter()
        .find(|(name, _)| *name == app)
        .map(|(_, directory)| *directory)
}

pub fn apps_in(directory: Directory) -> impl Iterator<Item = &'static str> {
    APP_DIRECTORIES
        .iter()
        .filter(move |(_, entry)| *entry == directory)
        .map(|(name, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_carries_forty_three_apps_with_unique_names() {
        assert_eq!(APP_DIRECTORIES.len(), 43);
        for (name, _) in APP_DIRECTORIES {
            assert_eq!(
                APP_DIRECTORIES.iter().filter(|(other, _)| *other == name).count(),
                1,
                "duplicate app in the table: {name}"
            );
        }
    }

    #[test]
    fn directory_names_fit_the_8_3_stem() {
        for directory in ALL_DIRECTORIES {
            assert!(
                !directory.name().is_empty() && directory.name().len() <= 8,
                "directory name exceeds the 8.3 stem: {}",
                directory.name()
            );
        }
    }

    #[test]
    fn directory_names_are_unique() {
        for directory in ALL_DIRECTORIES {
            assert_eq!(
                ALL_DIRECTORIES
                    .iter()
                    .filter(|other| other.name() == directory.name())
                    .count(),
                1
            );
        }
    }

    #[test]
    fn search_path_is_ordered_and_excludes_services_and_boot() {
        assert_eq!(
            SEARCH_PATH,
            [
                Directory::Bin,
                Directory::Tests,
                Directory::Games,
                Directory::Bench,
                Directory::Stress,
            ]
        );
        assert!(!SEARCH_PATH.contains(&Directory::Srv));
        assert!(!SEARCH_PATH.contains(&Directory::Boot));
    }

    #[test]
    fn directory_of_answers_every_table_app_and_refuses_strangers() {
        for (name, directory) in APP_DIRECTORIES {
            assert_eq!(directory_of(name), Some(directory), "{name}");
        }
        assert_eq!(directory_of("nosuchapp"), None);
        assert_eq!(directory_of(""), None);
    }

    #[test]
    fn apps_in_partitions_the_whole_table_exactly_once() {
        let total: usize = ALL_DIRECTORIES.iter().map(|d| apps_in(*d).count()).sum();
        assert_eq!(total, APP_DIRECTORIES.len());
    }

    #[test]
    fn bin_taxonomy_is_pinned() {
        assert_eq!(
            apps_in(Directory::Bin).collect::<Vec<&str>>(),
            [
                "mem", "ps", "vfs", "ata", "format", "mount", "attach", "rm", "cp", "mkdir",
                "rmdir", "pci", "ne2k", "arp", "ip", "ted", "shell",
            ]
        );
    }

    #[test]
    fn games_taxonomy_is_pinned() {
        assert_eq!(
            apps_in(Directory::Games).collect::<Vec<&str>>(),
            ["snake", "tetris", "conway"]
        );
    }

    #[test]
    fn tests_directory_taxonomy_is_pinned() {
        assert_eq!(
            apps_in(Directory::Tests).collect::<Vec<&str>>(),
            ["bulk", "ping", "threads", "waitset", "leaky", "exiter", "spinner"]
        );
    }

    #[test]
    fn bench_taxonomy_is_pinned() {
        assert_eq!(
            apps_in(Directory::Bench).collect::<Vec<&str>>(),
            ["bench", "fsbench"]
        );
    }

    #[test]
    fn stress_taxonomy_is_pinned() {
        assert_eq!(apps_in(Directory::Stress).collect::<Vec<&str>>(), ["pi", "stress"]);
    }

    #[test]
    fn srv_taxonomy_is_pinned() {
        assert_eq!(
            apps_in(Directory::Srv).collect::<Vec<&str>>(),
            [
                "randomd", "echo", "pong", "vfsd", "inputd", "ps2d", "atad", "blockd", "pcid",
                "ne2kd", "ipd",
            ]
        );
    }

    #[test]
    fn boot_taxonomy_is_pinned() {
        assert_eq!(apps_in(Directory::Boot).collect::<Vec<&str>>(), ["init"]);
    }
}
