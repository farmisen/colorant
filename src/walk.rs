//! Walk up from a starting directory looking for a named file.

use std::path::{Path, PathBuf};

/// Walk from `start` toward the filesystem root, returning the first path
/// where `<dir>/<filename>` is a regular file. Returns `None` if no such file
/// exists in any ancestor.
pub fn find_nearest(start: &Path, filename: &str) -> Option<PathBuf> {
    let mut dir = start;
    loop {
        let candidate = dir.join(filename);
        if candidate.is_file() {
            return Some(candidate);
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn finds_file_in_ancestor() {
        let root = tempdir().unwrap();
        let nested = root.path().join("a/b/c");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.path().join(".colorantrc"), "fg = #ffffff").unwrap();

        let found = find_nearest(&nested, ".colorantrc").unwrap();
        assert_eq!(found, root.path().join(".colorantrc"));
    }

    #[test]
    fn returns_none_when_missing() {
        let root = tempdir().unwrap();
        let nested = root.path().join("a/b");
        fs::create_dir_all(&nested).unwrap();
        assert!(find_nearest(&nested, ".colorantrc").is_none());
    }

    #[test]
    fn closest_ancestor_wins() {
        // Two ancestors carry a `.colorantrc`. From a leaf below the deeper
        // one, that deeper file must be returned — applying the wrong
        // project's theme when both a workspace and a sub-project have rcs
        // would silently break user expectations.
        let root = tempdir().unwrap();
        let mid = root.path().join("a");
        let leaf = mid.join("b/c");
        fs::create_dir_all(&leaf).unwrap();
        fs::write(root.path().join(".colorantrc"), "fg = #111111\n").unwrap();
        fs::write(mid.join(".colorantrc"), "fg = #222222\n").unwrap();

        let found = find_nearest(&leaf, ".colorantrc").unwrap();
        assert_eq!(found, mid.join(".colorantrc"));
    }
}
