//! Application state ("view model") – plain Rust without an egui dependency.
//!
//! All decisions (what is valid, what can be started, how to handle a job
//! result) live here, so they can be covered by ordinary unit tests. Rendering
//! in `views/` only reads this state and calls its methods.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pabaru_core::{
    Decoder, EncodeOptions, LayoutProposal, PageScan, Paper, Prepared, Printer, Profile, Progress,
    SuggestInput, suggest,
};

use crate::i18n::{self, tr};

/// State of a long-running job.
#[derive(Debug, Clone, Default)]
pub enum Task<T> {
    #[default]
    Idle,
    Running,
    Done(T),
    Failed(String),
}

impl<T> Task<T> {
    pub fn is_running(&self) -> bool {
        matches!(self, Task::Running)
    }
    pub fn done(&self) -> Option<&T> {
        match self {
            Task::Done(v) => Some(v),
            _ => None,
        }
    }
}

/// Application tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Backup,
    Restore,
}

/// Paper choice in the GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaperChoice {
    #[default]
    A4,
    Letter,
}

impl PaperChoice {
    pub fn paper(self) -> Paper {
        match self {
            PaperChoice::A4 => Paper::A4,
            PaperChoice::Letter => Paper::Letter,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            PaperChoice::A4 => "A4",
            PaperChoice::Letter => "Letter",
        }
    }
}

/// Offered scanner resolutions.
pub const SCANNER_DPIS: [u32; 4] = [300, 600, 1200, 2400];
/// Paper margin used by the GUI (mm).
pub const MARGIN_MM: f32 = 10.0;

/// Export result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSummary {
    pub path: PathBuf,
    pub pages: usize,
}

// ---------------------------------------------------------------------------
// Backup
// ---------------------------------------------------------------------------

/// State of the "Back up" tab.
#[derive(Debug, Clone)]
pub struct BackupModel {
    pub inputs: Vec<PathBuf>,
    /// Bumped on every input change; results of older jobs are discarded.
    pub generation: u64,
    pub prepared: Task<Arc<Prepared>>,
    pub printer: Printer,
    pub scanner_dpi: u32,
    pub paper: PaperChoice,
    pub encrypt: bool,
    pub password: String,
    pub password_again: String,
    pub selected: Profile,
    pub proposals: Vec<LayoutProposal>,
    pub export: Task<ExportSummary>,
}

impl Default for BackupModel {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            generation: 0,
            prepared: Task::Idle,
            printer: Printer::Laser600,
            scanner_dpi: 600,
            paper: PaperChoice::A4,
            encrypt: false,
            password: String::new(),
            password_again: String::new(),
            selected: Profile::Balanced,
            proposals: Vec::new(),
            export: Task::Idle,
        }
    }
}

/// Adds paths to the list without duplicates. Returns whether the list changed.
fn add_unique(list: &mut Vec<PathBuf>, paths: impl IntoIterator<Item = PathBuf>) -> bool {
    let mut changed = false;
    for p in paths {
        if !list.contains(&p) {
            list.push(p);
            changed = true;
        }
    }
    changed
}

impl BackupModel {
    /// Adds files/folders. Returns `true` if the data must be prepared again.
    pub fn add_inputs(&mut self, paths: impl IntoIterator<Item = PathBuf>) -> bool {
        let changed = add_unique(&mut self.inputs, paths);
        if changed {
            self.invalidate();
        }
        changed
    }

    /// Removes an input. Returns `true` if the data must be prepared again.
    pub fn remove_input(&mut self, index: usize) -> bool {
        if index >= self.inputs.len() {
            return false;
        }
        self.inputs.remove(index);
        self.invalidate();
        !self.inputs.is_empty()
    }

    pub fn clear(&mut self) {
        self.inputs.clear();
        self.invalidate();
    }

    /// After an input change: drop the prepared data and proposals.
    fn invalidate(&mut self) {
        self.generation += 1;
        self.prepared = if self.inputs.is_empty() {
            Task::Idle
        } else {
            Task::Running
        };
        self.proposals.clear();
        self.export = Task::Idle;
    }

    /// Result of data preparation (from a thread). Older generations are ignored.
    pub fn on_prepared(&mut self, generation: u64, result: Result<Prepared, String>) {
        if generation != self.generation {
            return;
        }
        self.prepared = match result {
            Ok(p) => Task::Done(Arc::new(p)),
            Err(e) => Task::Failed(e),
        };
        self.recompute();
    }

    /// Recomputes proposals (fast – called after every settings change).
    pub fn recompute(&mut self) {
        let Some(prepared) = self.prepared.done() else {
            self.proposals.clear();
            return;
        };
        self.proposals = suggest(&SuggestInput {
            stream_len: prepared.stream_len(self.encrypt),
            printer: self.printer,
            scanner_dpi: self.scanner_dpi,
            paper: self.paper.paper(),
            margin_mm: MARGIN_MM,
        });
        // The selected profile may no longer be available.
        if !self.proposals.iter().any(|p| p.profile == self.selected)
            && let Some(first) = self.proposals.first()
        {
            self.selected = first.profile;
        }
        self.export = Task::Idle;
    }

    pub fn selected_proposal(&self) -> Option<&LayoutProposal> {
        self.proposals.iter().find(|p| p.profile == self.selected)
    }

    /// Password problem that blocks the export (`None` = fine).
    pub fn password_problem(&self) -> Option<&'static str> {
        if !self.encrypt {
            return None;
        }
        if self.password.is_empty() {
            Some(tr("Enter a password.", "Zadejte heslo."))
        } else if self.password != self.password_again {
            Some(tr("Passwords do not match.", "Hesla se neshodují."))
        } else {
            None
        }
    }

    /// Password strength advice (does not block the export).
    pub fn password_hint(&self) -> Option<&'static str> {
        (self.encrypt && !self.password.is_empty() && self.password.chars().count() < 10)
            .then(|| {
                tr(
                    "We recommend at least 10 characters – the password protects the data for years.",
                    "Doporučujeme aspoň 10 znaků – heslo chrání data i po letech.",
                )
            })
    }

    /// Why the export is not possible (`None` = it is).
    pub fn export_blocker(&self) -> Option<&'static str> {
        if self.inputs.is_empty() {
            return Some(tr("Add files or folders.", "Přidejte soubory nebo složky."));
        }
        if self.prepared.is_running() {
            return Some(tr("Preparing data…", "Připravuji data…"));
        }
        if self.selected_proposal().is_none() {
            return Some(tr(
                "No layout fits the selected scanner and paper.",
                "Žádné rozložení nevyhovuje zvolenému skeneru a papíru.",
            ));
        }
        if self.export.is_running() {
            return Some(tr("Exporting…", "Probíhá export…"));
        }
        self.password_problem()
    }

    /// Encoding options for the selected proposal.
    pub fn encode_options(&self) -> Option<EncodeOptions> {
        let p = self.selected_proposal()?;
        Some(EncodeOptions {
            password: self.encrypt.then(|| self.password.clone()),
            ..EncodeOptions::from_proposal(p)
        })
    }

    /// Suggested file name for the export.
    pub fn suggested_file_name(&self) -> String {
        let base = self
            .prepared
            .done()
            .map(|p| p.default_name())
            .unwrap_or_else(|| tr("backup", "zaloha").into());
        let safe: String = base
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("pabaru-{safe}.pdf")
    }
}

// ---------------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------------

/// Summary of one decoded page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSummary {
    pub page_index: Option<u16>,
    pub ok: usize,
    pub failed: usize,
}

/// One file with scans.
#[derive(Debug, Clone)]
pub struct ScanEntry {
    pub path: PathBuf,
    pub state: Task<Vec<PageSummary>>,
}

impl ScanEntry {
    pub fn file_name(&self) -> String {
        file_name(&self.path)
    }
}

/// Where to restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DestChoice {
    /// Into the folder the (first) scan was loaded from.
    #[default]
    NextToScans,
    /// Into a folder picked by the user.
    Folder,
}

/// Restore result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreSummary {
    pub files: usize,
    pub recovered_blocks: usize,
    pub target: String,
}

/// State of the "Restore" tab.
#[derive(Debug, Clone, Default)]
pub struct RestoreModel {
    pub scans: Vec<ScanEntry>,
    pub decoder: Decoder,
    /// Decoded pages (label, scan) for the quality map.
    pub pages: Vec<(String, PageScan)>,
    pub selected_page: usize,
    pub password: String,
    pub dest: DestChoice,
    pub folder: Option<PathBuf>,
    pub restore: Task<RestoreSummary>,
}

impl RestoreModel {
    /// Adds scan files. Returns the new paths that need processing.
    pub fn add_scans(&mut self, paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
        let mut new = Vec::new();
        for p in paths {
            if self.scans.iter().any(|s| s.path == p) {
                continue;
            }
            self.scans.push(ScanEntry {
                path: p.clone(),
                state: Task::Running,
            });
            new.push(p);
        }
        if !new.is_empty() {
            self.restore = Task::Idle;
        }
        new
    }

    /// Result of decoding a scan file (from a thread).
    pub fn on_scan_decoded(&mut self, path: &Path, result: Result<Vec<PageScan>, String>) {
        let Some(idx) = self.scans.iter().position(|s| s.path == path) else {
            return;
        };
        match result {
            Ok(scans) => {
                let multi = scans.len() > 1;
                let mut summaries = Vec::new();
                for (n, scan) in scans.into_iter().enumerate() {
                    let report = self.decoder.add_scan(scan);
                    summaries.push(PageSummary {
                        page_index: report.page_index,
                        ok: report.ok_blocks,
                        failed: report.failed_blocks,
                    });
                    let label = if multi {
                        format!("{} · {}", file_name(path), n + 1)
                    } else {
                        file_name(path)
                    };
                    self.pages.push((label, report.scan));
                }
                self.scans[idx].state = Task::Done(summaries);
            }
            Err(e) => self.scans[idx].state = Task::Failed(e),
        }
    }

    pub fn clear(&mut self) {
        *self = Self {
            dest: self.dest,
            folder: self.folder.clone(),
            ..Self::default()
        };
    }

    /// Folder of the first scan (target for [`DestChoice::NextToScans`]).
    pub fn scans_dir(&self) -> Option<PathBuf> {
        self.scans
            .first()
            .and_then(|s| s.path.parent())
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
    }

    /// Effective restore folder according to the destination choice.
    pub fn target_dir(&self) -> Option<PathBuf> {
        match self.dest {
            DestChoice::NextToScans => self.scans_dir(),
            DestChoice::Folder => self.folder.clone(),
        }
    }

    pub fn progress(&self) -> Option<Progress> {
        self.decoder.progress()
    }

    pub fn busy(&self) -> bool {
        self.scans.iter().any(|s| s.state.is_running())
    }

    pub fn needs_password(&self) -> bool {
        self.decoder.needs_password()
    }

    /// The password, if one is needed.
    pub fn password_opt(&self) -> Option<String> {
        self.needs_password().then(|| self.password.clone())
    }

    /// Why the restore is not possible (`None` = it is).
    pub fn restore_blocker(&self) -> Option<String> {
        if self.scans.is_empty() {
            return Some(tr("Add scanned pages.", "Přidejte naskenované stránky.").into());
        }
        if self.busy() {
            return Some(tr("Processing scans…", "Zpracovávám skeny…").into());
        }
        let Some(p) = self.progress() else {
            return Some(
                tr(
                    "No Pabaru backup was found on the scans.",
                    "Na skenech nebyla nalezena záloha Pabaru.",
                )
                .into(),
            );
        };
        if !p.can_restore() {
            let missing: Vec<String> = p
                .missing_pages()
                .iter()
                .map(|n| (n + 1).to_string())
                .collect();
            let lacking = p.need.saturating_sub(p.have);
            let pages = missing.join(", ");
            return Some(match (i18n::lang(), missing.is_empty()) {
                (i18n::Lang::En, true) => {
                    format!("{lacking} blocks are missing – try a better scan.")
                }
                (i18n::Lang::En, false) => {
                    format!("Blocks are missing – scan pages {pages}.")
                }
                (i18n::Lang::Cs, true) => format!("Chybí {lacking} bloků – zkuste lepší sken."),
                (i18n::Lang::Cs, false) => {
                    format!("Chybí bloky – naskenujte stránky {pages}.")
                }
            });
        }
        if self.needs_password() && self.password.is_empty() {
            return Some(tr("Enter the backup password.", "Zadejte heslo zálohy.").into());
        }
        if self.target_dir().is_none() {
            return Some(tr("Choose a destination folder.", "Vyberte cílovou složku.").into());
        }
        if self.restore.is_running() {
            return Some(tr("Restoring…", "Obnovuji…").into());
        }
        None
    }
}

/// File name from a path (for display).
pub fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// Human-readable size (decimal separator follows the UI language).
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{} {}", i18n::decimal(v, 1), UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::i18n::{Lang, set_lang};

    fn prepared_for(dir: &Path) -> Prepared {
        std::fs::write(dir.join("a.txt"), "hello ".repeat(1000)).unwrap();
        pabaru_core::prepare(&[dir.to_path_buf()], 3).unwrap()
    }

    #[test]
    fn inputs_without_duplicates_and_generation() {
        let mut m = BackupModel::default();
        assert!(m.add_inputs([PathBuf::from("a"), PathBuf::from("b")]));
        assert!(!m.add_inputs([PathBuf::from("a")]));
        assert_eq!(m.inputs.len(), 2);
        assert!(m.prepared.is_running());
        let g = m.generation;
        assert!(m.remove_input(0));
        assert!(m.generation > g);
        assert!(
            !m.remove_input(0),
            "nothing to prepare after removing the last input"
        );
        assert!(matches!(m.prepared, Task::Idle));
    }

    #[test]
    fn stale_result_is_discarded_and_proposals_are_computed() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = BackupModel::default();
        m.add_inputs([dir.path().to_path_buf()]);
        let old = m.generation;
        m.add_inputs([PathBuf::from("other")]);
        m.on_prepared(old, Ok(prepared_for(dir.path())));
        assert!(
            m.prepared.is_running(),
            "a result of an old generation is ignored"
        );

        m.on_prepared(m.generation, Ok(prepared_for(dir.path())));
        assert_eq!(m.proposals.len(), 3);
        assert_eq!(m.selected, Profile::Balanced);
        assert!(m.export_blocker().is_none());
        assert!(m.encode_options().is_some());
    }

    #[test]
    fn passwords_must_match() {
        set_lang(Lang::En);
        let mut m = BackupModel {
            encrypt: true,
            ..Default::default()
        };
        assert_eq!(m.password_problem(), Some("Enter a password."));
        m.password = "abc".into();
        m.password_again = "abd".into();
        assert_eq!(m.password_problem(), Some("Passwords do not match."));
        m.password_again = "abc".into();
        assert!(m.password_problem().is_none());
        assert!(m.password_hint().is_some(), "short password → advice");
    }

    #[test]
    fn weak_scanner_changes_profile_choice() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = BackupModel::default();
        m.add_inputs([dir.path().to_path_buf()]);
        m.on_prepared(m.generation, Ok(prepared_for(dir.path())));
        m.scanner_dpi = 300;
        m.recompute();
        assert!(m.proposals.iter().all(|p| p.min_scan_dpi <= 300));
        assert!(m.selected_proposal().is_some());
    }

    #[test]
    fn restore_without_scans_is_blocked() {
        set_lang(Lang::En);
        let mut r = RestoreModel::default();
        assert!(r.restore_blocker().is_some());
        let new = r.add_scans([PathBuf::from("x.png"), PathBuf::from("x.png")]);
        assert_eq!(new.len(), 1);
        assert!(r.busy());
        r.on_scan_decoded(Path::new("x.png"), Err("unreadable".into()));
        assert!(!r.busy());
        assert!(matches!(r.scans[0].state, Task::Failed(_)));
        assert!(r.restore_blocker().unwrap().contains("was found"));
    }

    #[test]
    fn sizes() {
        set_lang(Lang::En);
        assert_eq!(human_size(10), "10 B");
        assert_eq!(human_size(1536), "1.5 KB");
    }
}
