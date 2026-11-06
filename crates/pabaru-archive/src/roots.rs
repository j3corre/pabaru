//! Naming of the inputs (roots) inside the archive.
//!
//! The user can drop several files and folders at once, even from different
//! locations or drives. We need:
//!
//! 1. two folders with the same name (e.g. `C:\work\docs` and
//!    `D:\backup\docs`) must **not overwrite** each other in the archive,
//! 2. their relative position (relative structure) must be preserved.
//!
//! The absolute disk path is used only while packing; it is not stored
//! in the archive.
//!
//! Solution: find the deepest common ancestor of all inputs and name them
//! in the archive relative to it. On Windows, inputs from different drives
//! have no common ancestor, so the name starts with the drive letter
//! (`C/work/docs`, `D/backup/docs`).

use std::path::{Component, Path, PathBuf};

use crate::ArchiveError;

/// A single archive root (input).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    /// Absolute path on disk.
    pub disk: PathBuf,
    /// Name inside the archive (`/` separator).
    pub name: String,
}

/// Splits an absolute path into text parts. Drive `C:` becomes `C`,
/// the root directory is omitted.
fn parts(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Prefix(p) => Some(
                p.as_os_str()
                    .to_string_lossy()
                    .trim_start_matches(r"\\?\")
                    .trim_end_matches(':')
                    .replace(['\\', ':'], "_"),
            ),
            Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// Converts the inputs into roots with unique names.
pub fn resolve_roots(inputs: &[PathBuf]) -> Result<Vec<Root>, ArchiveError> {
    // 1) Absolute paths, sorted and deduplicated.
    let mut abs: Vec<PathBuf> = inputs
        .iter()
        .map(std::path::absolute)
        .collect::<Result<_, _>>()?;
    abs.sort();
    abs.dedup();

    // 2) Drop inputs that lie inside another input (otherwise their
    //    content would be packed twice).
    let abs: Vec<PathBuf> = abs
        .iter()
        .filter(|p| !abs.iter().any(|q| q != *p && p.starts_with(q)))
        .cloned()
        .collect();

    let all_parts: Vec<Vec<String>> = abs.iter().map(|p| parts(p)).collect();
    for (p, ps) in abs.iter().zip(&all_parts) {
        if ps.is_empty() {
            return Err(ArchiveError::InvalidInput(p.clone()));
        }
    }

    // 3) Length of the common ancestor (computed from the parents, so that
    //    every input keeps at least its own name).
    let common = if all_parts.len() == 1 {
        all_parts[0].len() - 1
    } else {
        let first = &all_parts[0];
        let mut n = 0;
        'outer: while n < first.len() - 1 {
            for ps in &all_parts[1..] {
                if n >= ps.len() - 1 || ps[n] != first[n] {
                    break 'outer;
                }
            }
            n += 1;
        }
        n
    };

    Ok(abs
        .into_iter()
        .zip(all_parts)
        .map(|(disk, ps)| Root {
            disk,
            name: ps[common..].join("/"),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(inputs: &[&str]) -> Vec<String> {
        let v: Vec<PathBuf> = inputs.iter().map(PathBuf::from).collect();
        resolve_roots(&v)
            .unwrap()
            .into_iter()
            .map(|r| r.name)
            .collect()
    }

    #[cfg(windows)]
    #[test]
    fn windows_different_drives_and_same_names() {
        assert_eq!(
            names(&[r"C:\work\docs", r"D:\backup\docs"]),
            vec!["C/work/docs", "D/backup/docs"]
        );
        assert_eq!(
            names(&[r"C:\a\x\docs", r"C:\a\y\docs"]),
            vec!["x/docs", "y/docs"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_same_names() {
        assert_eq!(
            names(&["/home/a/x/docs", "/home/a/y/docs"]),
            vec!["x/docs", "y/docs"]
        );
    }

    #[test]
    fn single_input_has_only_its_own_name() {
        let base = std::env::temp_dir().join("pabaru-test").join("photos");
        assert_eq!(names(&[base.to_str().unwrap()]), vec!["photos"]);
    }

    #[test]
    fn siblings_and_nested_input() {
        let base = std::env::temp_dir().join("pabaru-test");
        let a = base.join("a");
        let b = base.join("b");
        let inner = a.join("nested");
        assert_eq!(
            names(&[
                a.to_str().unwrap(),
                b.to_str().unwrap(),
                inner.to_str().unwrap()
            ]),
            vec!["a", "b"]
        );
    }
}
