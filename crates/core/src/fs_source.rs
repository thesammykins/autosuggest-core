//! Filesystem suggestion source for `filepaths`/`folders` templates.
//!
//! This is the **only** place in `core` permitted to touch the filesystem
//! (`TECH.md §2` makes an explicit exception for the M1 path templates). It is
//! deliberately isolated here so `tokenize` and `parse` remain pure. All reads
//! are best-effort: a missing or unreadable directory yields an empty list
//! rather than an error or panic.

use std::path::{Path, PathBuf};

/// What kind of filesystem entries to suggest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsKind {
    /// Files and directories (`template: "filepaths"`).
    FilesAndDirs,
    /// Directories only (`template: "folders"`).
    DirsOnly,
}

/// A single filesystem suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsEntry {
    /// The name to insert (directories carry a trailing `/`). When the query had
    /// a directory prefix (e.g. `src/m`), the prefix is preserved so the insert
    /// is a usable path (`src/main.rs`).
    pub insert: String,
    /// The bare entry name for display (without the leading directory prefix).
    pub display: String,
    /// Whether the entry is a directory.
    pub is_dir: bool,
}

/// List filesystem entries matching `partial` relative to `cwd`.
///
/// `partial` is the in-progress path the user typed (e.g. `""`, `src/`,
/// `src/ma`). The directory component selects which folder to read; the file
/// component is the prefix entries must start with. Directories are suffixed
/// with `/`. Hidden entries (leading `.`) are included only when the file
/// component itself starts with `.`.
///
/// Returns an empty vector if the resolved directory cannot be read.
pub fn list_entries(cwd: &Path, partial: &str, kind: FsKind) -> Vec<FsEntry> {
    let (dir_prefix, file_prefix) = split_partial(partial);

    // Resolve the directory to read. A leading `~` or absolute path is honoured;
    // otherwise the directory is relative to `cwd`.
    let read_dir = resolve_dir(cwd, dir_prefix);

    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&read_dir) {
        Ok(e) => e,
        Err(_) => return out, // missing/unreadable dir => no suggestions
    };

    let want_hidden = file_prefix.starts_with('.');

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if name.starts_with('.') && !want_hidden {
            continue;
        }
        if !name.starts_with(file_prefix) {
            continue;
        }

        let is_dir = entry
            .file_type()
            .map(|t| t.is_dir())
            .unwrap_or_else(|_| read_dir.join(&*name).is_dir());

        if kind == FsKind::DirsOnly && !is_dir {
            continue;
        }

        let mut insert = String::new();
        insert.push_str(dir_prefix);
        insert.push_str(&name);
        if is_dir {
            insert.push('/');
        }

        out.push(FsEntry {
            insert,
            display: name.into_owned(),
            is_dir,
        });
    }

    // Deterministic order: directories first, then lexicographic.
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.display.cmp(&b.display))
    });
    out
}

/// Split a partial path into `(directory_prefix, file_prefix)`.
///
/// The directory prefix retains its trailing slash so it can be re-prepended to
/// the inserted value. `"src/ma"` -> `("src/", "ma")`; `"file"` -> `("", "file")`.
fn split_partial(partial: &str) -> (&str, &str) {
    match partial.rfind('/') {
        Some(pos) => (&partial[..=pos], &partial[pos + 1..]),
        None => ("", partial),
    }
}

/// Resolve the directory to read from `cwd` and a (possibly empty) directory
/// prefix. Handles absolute paths and a leading `~` (home) where available.
fn resolve_dir(cwd: &Path, dir_prefix: &str) -> PathBuf {
    if dir_prefix.is_empty() {
        return cwd.to_path_buf();
    }
    if let Some(rest) = dir_prefix.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    if dir_prefix == "~" || dir_prefix == "~/" {
        if let Some(home) = home_dir() {
            return home;
        }
    }
    let p = Path::new(dir_prefix);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

/// Best-effort home directory from the environment (no extra dependencies).
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "asc_fs_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&d).expect("mk temp");
        d
    }

    #[test]
    fn lists_files_and_dirs_with_slash_suffix() {
        let d = temp_dir("mix");
        fs::write(d.join("a.txt"), "x").expect("write");
        fs::create_dir(d.join("sub")).expect("mkdir");
        let mut got = list_entries(&d, "", FsKind::FilesAndDirs);
        got.sort_by(|a, b| a.insert.cmp(&b.insert));
        let inserts: Vec<&str> = got.iter().map(|e| e.insert.as_str()).collect();
        assert!(inserts.contains(&"a.txt"));
        assert!(inserts.contains(&"sub/"));
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn folders_only_excludes_files() {
        let d = temp_dir("folders");
        fs::write(d.join("a.txt"), "x").expect("write");
        fs::create_dir(d.join("sub")).expect("mkdir");
        let got = list_entries(&d, "", FsKind::DirsOnly);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].insert, "sub/");
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn respects_file_prefix() {
        let d = temp_dir("prefix");
        fs::write(d.join("alpha"), "x").expect("write");
        fs::write(d.join("beta"), "x").expect("write");
        let got = list_entries(&d, "al", FsKind::FilesAndDirs);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].insert, "alpha");
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn hidden_only_when_dot_typed() {
        let d = temp_dir("hidden");
        fs::write(d.join(".secret"), "x").expect("write");
        fs::write(d.join("visible"), "x").expect("write");
        let without = list_entries(&d, "", FsKind::FilesAndDirs);
        assert!(without.iter().all(|e| e.insert != ".secret"));
        let with = list_entries(&d, ".", FsKind::FilesAndDirs);
        assert!(with.iter().any(|e| e.insert == ".secret"));
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn nested_prefix_preserved_in_insert() {
        let d = temp_dir("nested");
        fs::create_dir(d.join("src")).expect("mkdir");
        fs::write(d.join("src").join("main.rs"), "x").expect("write");
        let got = list_entries(&d, "src/ma", FsKind::FilesAndDirs);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].insert, "src/main.rs");
        assert_eq!(got[0].display, "main.rs");
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn missing_dir_returns_empty() {
        let d = temp_dir("missing");
        let got = list_entries(&d, "does_not_exist/", FsKind::FilesAndDirs);
        assert!(got.is_empty());
        fs::remove_dir_all(&d).ok();
    }
}
