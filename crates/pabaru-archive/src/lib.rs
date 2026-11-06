//! # pabaru-archive
//!
//! Packs files and whole folders into a single compressed stream and can
//! unpack them again.
//!
//! Stream format: `zstd( tar( manifest + files ) )`.
//!
//! * **tar** preserves the folder structure, names and modification times,
//! * **zstd** shrinks the data (less paper),
//! * **manifest** (`.pabaru/manifest.json`, the first tar entry) contains the
//!   BLAKE3 hash of every file, so after a restore we can verify everything is intact.
//!   It also lists the archive roots (dropped folders/files) by name only –
//!   original absolute paths are deliberately NOT stored (privacy).
//!
//! Several folders can be packed at once, see the [`roots`] module.

pub mod roots;

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use walkdir::WalkDir;

/// Path of the manifest inside the archive.
pub const MANIFEST_PATH: &str = ".pabaru/manifest.json";

/// Archiving errors.
#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("folder traversal error: {0}")]
    Walk(#[from] walkdir::Error),
    #[error("manifest error: {0}")]
    Manifest(#[from] serde_json::Error),
    #[error("input {0} has no valid name")]
    InvalidInput(PathBuf),
    #[error("archive contains no manifest")]
    MissingManifest,
    #[error("file {0} has a different hash than recorded in the manifest")]
    HashMismatch(String),
    #[error("unsafe or unknown path in archive: {0}")]
    UnsafePath(String),
}

/// A single manifest entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Relative path within the archive (`/` separator).
    pub path: String,
    pub size: u64,
    /// BLAKE3 hash of the content in hexadecimal.
    pub blake3: String,
}

/// An archive root – one dropped file or folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRoot {
    /// Name inside the archive (e.g. `x/docs`).
    pub name: String,
}

/// Manifest – the list of files in the archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Manifest {
    pub version: u32,
    #[serde(default)]
    pub roots: Vec<ManifestRoot>,
    pub files: Vec<ManifestEntry>,
}

/// Where to restore files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination<'a> {
    /// Into the given folder (the relative structure of roots is kept).
    Folder(&'a Path),
}

/// Quick summary of the inputs (for the GUI after a folder is dropped).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InputSummary {
    pub file_count: u64,
    pub dir_count: u64,
    pub total_bytes: u64,
}

/// Result of packing.
#[derive(Debug, Clone)]
pub struct Packed {
    /// The resulting compressed stream.
    pub stream: Vec<u8>,
    pub manifest: Manifest,
    /// Size before compression (sum of file sizes).
    pub raw_bytes: u64,
}

/// Result of unpacking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpackReport {
    /// Number of restored and verified files.
    pub verified_files: usize,
    pub manifest: Manifest,
}

/// One item to pack: its path on disk and its path in the archive.
struct Item {
    disk: PathBuf,
    name: String,
    is_dir: bool,
}

/// Walks the inputs and returns a sorted list of items (deterministic order)
/// together with the roots for the manifest.
fn collect_items(inputs: &[PathBuf]) -> Result<(Vec<Item>, Vec<ManifestRoot>), ArchiveError> {
    let roots = roots::resolve_roots(inputs)?;
    let mut items = Vec::new();
    for root in &roots {
        for entry in WalkDir::new(&root.disk)
            .follow_links(false)
            .sort_by_file_name()
        {
            let entry = entry?;
            let ft = entry.file_type();
            // Symbolic links and special files are skipped.
            if !ft.is_file() && !ft.is_dir() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&root.disk)
                .expect("path is under the input");
            let mut name = root.name.clone();
            for part in rel.components() {
                name.push('/');
                name.push_str(&part.as_os_str().to_string_lossy());
            }
            items.push(Item {
                disk: entry.path().to_path_buf(),
                name,
                is_dir: ft.is_dir(),
            });
        }
    }
    let manifest_roots = roots
        .into_iter()
        .map(|r| ManifestRoot { name: r.name })
        .collect();
    Ok((items, manifest_roots))
}

/// Quickly counts the files and total size (without reading contents).
pub fn summarize(inputs: &[PathBuf]) -> Result<InputSummary, ArchiveError> {
    let mut s = InputSummary::default();
    for item in collect_items(inputs)?.0 {
        if item.is_dir {
            s.dir_count += 1;
        } else {
            s.file_count += 1;
            s.total_bytes += fs::metadata(&item.disk)?.len();
        }
    }
    Ok(s)
}

/// Packs files and folders into a compressed stream.
///
/// `level` is the zstd compression level (1–22, 19 recommended).
pub fn pack(inputs: &[PathBuf], level: i32) -> Result<Packed, ArchiveError> {
    let (items, roots) = collect_items(inputs)?;

    // 1) Read the files and compute hashes for the manifest.
    let mut manifest = Manifest {
        version: 1,
        roots,
        files: Vec::new(),
    };
    let mut contents: Vec<Option<Vec<u8>>> = Vec::with_capacity(items.len());
    let mut raw_bytes = 0u64;
    for item in &items {
        if item.is_dir {
            contents.push(None);
            continue;
        }
        let data = fs::read(&item.disk)?;
        raw_bytes += data.len() as u64;
        manifest.files.push(ManifestEntry {
            path: item.name.clone(),
            size: data.len() as u64,
            blake3: blake3::hash(&data).to_hex().to_string(),
        });
        contents.push(Some(data));
    }

    // 2) Build the tar: manifest first, then folders and files.
    let mut builder = tar::Builder::new(Vec::new());
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest_json.len() as u64);
    header.set_mode(0o644);
    builder.append_data(&mut header, MANIFEST_PATH, manifest_json.as_slice())?;

    for (item, data) in items.iter().zip(contents) {
        let meta = fs::metadata(&item.disk)?;
        let mut header = tar::Header::new_gnu();
        header.set_metadata(&meta);
        match data {
            None => {
                header.set_size(0);
                builder.append_data(&mut header, &item.name, std::io::empty())?;
            }
            Some(bytes) => {
                header.set_size(bytes.len() as u64);
                builder.append_data(&mut header, &item.name, bytes.as_slice())?;
            }
        }
    }
    let tar_bytes = builder.into_inner()?;

    // 3) Compression.
    let stream = zstd::encode_all(Cursor::new(tar_bytes), level)?;
    Ok(Packed {
        stream,
        manifest,
        raw_bytes,
    })
}

/// Unpacks the stream into folder `dest` and verifies the hashes of all files.
pub fn unpack(stream: &[u8], dest: &Path) -> Result<UnpackReport, ArchiveError> {
    unpack_to(stream, Destination::Folder(dest))
}

/// Checks that an archive path is safe (only normal components, no `..`
/// and no absolute paths) and returns it as a relative `PathBuf`.
fn safe_relative(path: &str) -> Result<PathBuf, ArchiveError> {
    let p = Path::new(path);
    if path.is_empty() || !p.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(ArchiveError::UnsafePath(path.to_string()));
    }
    Ok(p.to_path_buf())
}

/// Computes the on-disk target path for an archive entry.
fn target_path(archive_path: &str, dest: Destination) -> Result<PathBuf, ArchiveError> {
    match dest {
        Destination::Folder(dir) => Ok(dir.join(safe_relative(archive_path)?)),
    }
}

/// Reads the manifest from the first tar entry.
fn take_manifest<R: Read>(entries: &mut tar::Entries<R>) -> Result<Manifest, ArchiveError> {
    let mut entry = entries.next().ok_or(ArchiveError::MissingManifest)??;
    if entry.path()?.to_string_lossy().replace('\\', "/") != MANIFEST_PATH {
        return Err(ArchiveError::MissingManifest);
    }
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;
    Ok(serde_json::from_slice(&buf)?)
}

/// Reads only the manifest (list of files and roots) without unpacking.
pub fn read_manifest(stream: &[u8]) -> Result<Manifest, ArchiveError> {
    let tar_bytes = zstd::decode_all(Cursor::new(stream))?;
    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
    take_manifest(&mut archive.entries()?)
}

/// Unpacks the stream to the chosen destination and verifies the hashes of all files.
pub fn unpack_to(stream: &[u8], dest: Destination) -> Result<UnpackReport, ArchiveError> {
    let Destination::Folder(dir) = dest;
    fs::create_dir_all(dir)?;
    let tar_bytes = zstd::decode_all(Cursor::new(stream))?;
    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));

    // The manifest is always the first entry – without it we cannot map paths.
    let mut entries = archive.entries()?;
    let manifest = take_manifest(&mut entries)?;

    for entry in entries {
        let mut entry = entry?;
        let raw = entry.path()?.to_string_lossy().replace('\\', "/");
        let path = raw.trim_end_matches('/');
        let target = target_path(path, dest)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        entry.set_preserve_mtime(true);
        entry.unpack(&target)?;
    }

    // Verify the hashes of the restored files.
    for f in &manifest.files {
        let disk = target_path(&f.path, dest)?;
        let data = fs::read(&disk)?;
        if blake3::hash(&data).to_hex().as_str() != f.blake3 {
            return Err(ArchiveError::HashMismatch(f.path.clone()));
        }
    }
    Ok(UnpackReport {
        verified_files: manifest.files.len(),
        manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tree(root: &Path) -> PathBuf {
        let dir = root.join("project");
        fs::create_dir_all(dir.join("sub/deeper")).unwrap();
        fs::create_dir_all(dir.join("empty")).unwrap();
        fs::write(dir.join("a.txt"), "Hello, paper!").unwrap();
        fs::write(dir.join("sub/b.bin"), vec![7u8; 10_000]).unwrap();
        fs::write(dir.join("sub/deeper/č.txt"), "diacritics").unwrap();
        dir
    }

    #[test]
    fn round_trip_folder() {
        let src = tempfile::tempdir().unwrap();
        let dir = make_tree(src.path());
        let packed = pack(&[dir], 19).unwrap();
        assert_eq!(packed.manifest.files.len(), 3);
        assert!(packed.stream.len() < 2_000, "compression does not work");

        let out = tempfile::tempdir().unwrap();
        let report = unpack(&packed.stream, out.path()).unwrap();
        assert_eq!(report.verified_files, 3);
        assert_eq!(
            fs::read_to_string(out.path().join("project/sub/deeper/č.txt")).unwrap(),
            "diacritics"
        );
        assert!(out.path().join("project/empty").is_dir());
    }

    #[test]
    fn single_file_and_summary() {
        let src = tempfile::tempdir().unwrap();
        let f = src.path().join("x.txt");
        fs::write(&f, "123").unwrap();
        let s = summarize(std::slice::from_ref(&f)).unwrap();
        assert_eq!((s.file_count, s.total_bytes), (1, 3));
        let packed = pack(&[f], 3).unwrap();
        let out = tempfile::tempdir().unwrap();
        unpack(&packed.stream, out.path()).unwrap();
        assert_eq!(fs::read_to_string(out.path().join("x.txt")).unwrap(), "123");
    }

    #[test]
    fn packing_is_deterministic() {
        let src = tempfile::tempdir().unwrap();
        let dir = make_tree(src.path());
        let a = pack(std::slice::from_ref(&dir), 19).unwrap();
        let b = pack(&[dir], 19).unwrap();
        assert_eq!(a.stream, b.stream);
    }

    #[test]
    fn multiple_folders_with_same_name() {
        let src = tempfile::tempdir().unwrap();
        for (sub, text) in [("x", "první"), ("y", "druhá")] {
            let d = src.path().join(sub).join("docs");
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("f.txt"), text).unwrap();
        }
        let inputs = vec![src.path().join("x/docs"), src.path().join("y/docs")];
        let packed = pack(&inputs, 3).unwrap();
        assert_eq!(packed.manifest.roots.len(), 2);

        let out = tempfile::tempdir().unwrap();
        unpack(&packed.stream, out.path()).unwrap();
        assert_eq!(
            fs::read_to_string(out.path().join("x/docs/f.txt")).unwrap(),
            "první"
        );
        assert_eq!(
            fs::read_to_string(out.path().join("y/docs/f.txt")).unwrap(),
            "druhá"
        );
    }

    #[test]
    fn manifest_has_no_absolute_paths() {
        let src = tempfile::tempdir().unwrap();
        let dir = make_tree(src.path());
        let packed = pack(std::slice::from_ref(&dir), 3).unwrap();
        let json = serde_json::to_string(&read_manifest(&packed.stream).unwrap()).unwrap();
        // The temp directory name is part of the source path only.
        let parent = src.path().file_name().unwrap().to_string_lossy();
        assert!(
            !json.contains(&*parent),
            "manifest leaks the source path: {json}"
        );
        assert!(!json.contains("original"));
    }

    #[test]
    fn old_manifest_with_original_paths_still_reads() {
        let json = r#"{"version":1,"roots":[{"name":"a","original":"C:/x/a"}],"files":[]}"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.roots[0].name, "a");
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        assert!(safe_relative("../etc/passwd").is_err());
        assert!(safe_relative("/abs").is_err());
        assert!(safe_relative("ok/path.txt").is_ok());
    }
}
