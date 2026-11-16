//! Per-language alias loader: extra spoken phrases per command token template.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Key: command token template. Value: extra phrases in grammar syntax.
pub type AliasMap = HashMap<String, Vec<String>>;

/// Loads every `*.toml` file in `dir` and merges the phrases per token template.
///
/// A missing directory gives an empty map. A file that cannot be read or does
/// not parse as `token -> [phrases]` is skipped as a whole.
pub fn load_aliases(dir: &Path) -> AliasMap {
    merge_aliases(&load_alias_files(dir))
}

/// Merges the maps of `load_alias_files` per token template. An exact
/// duplicate phrase is kept one time.
pub fn merge_aliases(files: &[(PathBuf, AliasMap)]) -> AliasMap {
    let mut map = AliasMap::new();
    for (_, file) in files {
        for (token, phrases) in file {
            let entry = map.entry(token.clone()).or_default();
            for phrase in phrases {
                if !entry.contains(phrase) {
                    entry.push(phrase.clone());
                }
            }
        }
    }
    map
}

/// The files that `load_aliases` merges, each with its own map, in the
/// order of the file names. The same files are skipped.
pub fn load_alias_files(dir: &Path) -> Vec<(PathBuf, AliasMap)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("toml"))
        })
        .collect();
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    files
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let file = toml::from_str::<AliasMap>(&text).ok()?;
            Some((path, file))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SET_DEST: &str = r#"SailTo:"{TXT:HarbourName}""#;

    fn fixtures() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/aliases")
    }

    #[test]
    fn test_loads_language_aliases() {
        let map = load_aliases(&fixtures());
        let phrases = &map[SET_DEST];
        assert!(phrases.contains(&"kurs setzen nach <System>".to_string()));
        assert_eq!(map["OpenChartLogPanel"].len(), 2);
    }

    #[test]
    fn merges_languages_and_dedups_exact_duplicates() {
        let map = load_aliases(&fixtures());
        // de.toml first (sorted), then fr.toml; the duplicate phrase is kept once.
        assert_eq!(
            map[SET_DEST],
            vec![
                "kurs setzen nach <System>".to_string(),
                "cap sur le port <System>".to_string(),
            ]
        );
        assert_eq!(
            map["OpenChartLogPanel"],
            vec![
                "kartenbuch [öffnen; zeigen;]".to_string(),
                "ouvrir le journal des cartes".to_string(),
            ]
        );
    }

    #[test]
    fn skips_malformed_file_but_loads_others() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.toml"), "this is = = not toml").unwrap();
        std::fs::write(dir.path().join("b.toml"), "'X' = 5\n").unwrap();
        std::fs::write(dir.path().join("c.TOML"), "'Y' = [\"ja\"]\n").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "'Z' = [\"no\"]\n").unwrap();
        let map = load_aliases(dir.path());
        assert_eq!(map.len(), 1);
        assert_eq!(map["Y"], vec!["ja".to_string()]);
    }

    #[test]
    fn test_alias_files_keep_each_file_apart() {
        let files = load_alias_files(&fixtures());
        let names: Vec<_> = files
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["de.toml", "fr.toml"]);
        assert_eq!(
            files[0].1[SET_DEST],
            vec!["kurs setzen nach <System>".to_string()]
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.toml"), "this is = = not toml").unwrap();
        std::fs::write(
            dir.path().join("c.TOML"),
            "'Y' = [\"ja\"]
",
        )
        .unwrap();
        let files = load_alias_files(dir.path());
        assert_eq!(files.len(), 1);
        assert!(files[0].0.ends_with("c.TOML"));
    }

    #[test]
    fn missing_directory_gives_empty_map() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_aliases(&dir.path().join("nope")).is_empty());
    }

    #[test]
    fn seed_file_parses_and_holds_example() {
        let seed = Path::new(env!("CARGO_MANIFEST_DIR")).join("aliases");
        let map = load_aliases(&seed);
        // The shape of the shipped seed file, not its real token: one or
        // more keys, and each key has one or more phrases.
        assert!(!map.is_empty());
        assert!(map.iter().all(|(key, phrases)| !key.is_empty()
            && !phrases.is_empty()
            && phrases.iter().all(|p| !p.trim().is_empty())));
    }
}
