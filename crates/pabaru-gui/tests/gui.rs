//! GUI tests via `egui_kittest`: real rendering of the app, clicking on
//! elements by their labels (AccessKit) and simulated drag & drop.
//!
//! System dialogs are replaced by [`FakeDialogs`] with fixed paths.
//! All tests here run in English; Czech is covered in `gui_cs.rs`
//! (a separate process, because the language is a global setting).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pabaru_core::{EncodeOptions, KdfConfig, Paper, PrintParams, Profile, encode};
use pabaru_gui::i18n::{Lang, set_lang};
use pabaru_gui::model::{Tab, Task};
use pabaru_gui::{Dialogs, PabaruApp};

/// Dialogs for tests – return fixed paths.
#[derive(Default, Clone)]
struct FakeDialogs {
    save_pdf: Option<PathBuf>,
    folder: Option<PathBuf>,
}

impl Dialogs for FakeDialogs {
    fn pick_files(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn pick_folders(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn pick_scans(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn pick_folder(&self, _title: &str) -> Option<PathBuf> {
        self.folder.clone()
    }
    fn save_pdf(&self, _name: &str) -> Option<PathBuf> {
        self.save_pdf.clone()
    }
}

/// A file "dropped" onto the window.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn harness(dialogs: FakeDialogs) -> Harness<'static, PabaruApp> {
    set_lang(Lang::En);
    Harness::builder()
        .with_size(egui::vec2(1240.0, 900.0))
        .build_eframe(move |cc| PabaruApp::with_dialogs(&cc.egui_ctx, Box::new(dialogs)))
}

/// Simulates dropping files onto the window.
fn drop_files(h: &mut Harness<'static, PabaruApp>, paths: &[PathBuf]) {
    for p in paths {
        h.input_mut()
            .dropped_files
            .push(Arc::new(Dropped(p.clone())));
    }
    h.step();
}

/// Waits for background jobs to finish and repaints.
fn wait(h: &mut Harness<'static, PabaruApp>) {
    for _ in 0..1500 {
        h.step();
        if h.state().pending_jobs() == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    h.run_steps(3);
}

fn make_folder(root: &Path) -> PathBuf {
    let dir = root.join("family-photos");
    std::fs::create_dir_all(dir.join("2024")).unwrap();
    std::fs::write(dir.join("list.txt"), "Photos from the trip\n".repeat(40)).unwrap();
    std::fs::write(dir.join("2024/note.md"), "# Note\nBackup to paper.\n").unwrap();
    dir
}

#[test]
fn start_screen_and_tab_switching() {
    let mut h = harness(FakeDialogs::default());
    h.run_steps(2);
    h.get_by_label("Drop files or folders here");
    h.get_by_label("Restore").click();
    h.run_steps(2);
    assert_eq!(h.state().tab, Tab::Restore);
    h.get_by_label("Drop scanned pages here");
}

#[test]
fn drag_and_drop_folder_profile_choice_and_pdf_export() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = make_folder(tmp.path());
    let pdf = tmp.path().join("output.pdf");
    let mut h = harness(FakeDialogs {
        save_pdf: Some(pdf.clone()),
        ..Default::default()
    });
    h.run_steps(2);

    drop_files(&mut h, std::slice::from_ref(&folder));
    assert_eq!(h.state().backup.inputs, vec![folder.clone()]);
    wait(&mut h);

    // After the data is prepared three proposals are shown.
    assert_eq!(h.state().backup.proposals.len(), 3);
    h.get_by_label("Profile Compact");
    h.get_by_label("Profile Balanced");
    h.get_by_label("Profile Archival").click();
    h.run_steps(2);
    assert_eq!(h.state().backup.selected, Profile::Archival);

    // The button may be below the window edge – scroll to it first.
    h.get_by_label("Save PDF…").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Save PDF…").click();
    h.run_steps(1);
    wait(&mut h);
    match &h.state().backup.export {
        Task::Done(s) => {
            assert_eq!(s.path, pdf);
            assert!(s.pages >= 2, "the archival profile has at least 2 pages");
        }
        other => panic!("export did not succeed: {other:?}"),
    }
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"));
    h.get_by_label_contains("Saved");
}

#[test]
fn removing_an_input() {
    let tmp = tempfile::tempdir().unwrap();
    let a = make_folder(tmp.path());
    let b = tmp.path().join("other.txt");
    std::fs::write(&b, "x").unwrap();
    let mut h = harness(FakeDialogs::default());
    drop_files(&mut h, &[a.clone(), b]);
    wait(&mut h);
    h.get_by_label("Remove other.txt").click();
    h.run_steps(1);
    wait(&mut h);
    assert_eq!(h.state().backup.inputs, vec![a]);
    assert_eq!(h.state().backup.proposals.len(), 3);
}

/// Creates a backup as PNG pages (small paper for speed).
fn make_scans(root: &Path, password: Option<&str>) -> (PathBuf, Vec<PathBuf>) {
    let folder = make_folder(root);
    let opts = EncodeOptions {
        params: PrintParams {
            paper: Paper::Custom {
                width_mm: 90.0,
                height_mm: 80.0,
            },
            margin_mm: 4.0,
            header_mm: 2.0,
            ..Default::default()
        },
        password: password.map(String::from),
        kdf: KdfConfig {
            mem_kib: 256,
            iterations: 1,
            parallelism: 1,
        },
        ..Default::default()
    };
    let backup = encode(std::slice::from_ref(&folder), &opts).unwrap();
    let scans = backup.write_pngs(&root.join("scans"), "scan").unwrap();
    (folder, scans)
}

#[test]
fn restore_from_dropped_scans() {
    let tmp = tempfile::tempdir().unwrap();
    let (folder, scans) = make_scans(tmp.path(), None);
    let out = tmp.path().join("restored");
    let mut h = harness(FakeDialogs {
        folder: Some(out.clone()),
        ..Default::default()
    });
    h.get_by_label("Restore").click();
    h.run_steps(1);
    drop_files(&mut h, &scans);
    wait(&mut h);

    let progress = h.state().restore.progress().expect("backup recognised");
    assert!(progress.can_restore());
    h.get_by_label("Quality map");
    h.get_by_label("Into a chosen folder").click();
    h.run_steps(1);
    h.get_by_label("Choose…").click();
    h.run_steps(1);
    h.get_by_label("Restore files").click();
    h.run_steps(1);
    wait(&mut h);
    assert!(
        matches!(h.state().restore.restore, Task::Done(_)),
        "{:?}",
        h.state().restore.restore
    );
    assert_eq!(
        std::fs::read(out.join("family-photos/list.txt")).unwrap(),
        std::fs::read(folder.join("list.txt")).unwrap()
    );
}

#[test]
fn encrypted_backup_requires_password() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, scans) = make_scans(tmp.path(), Some("secret-password"));
    // Default destination: next to the scans.
    let out = scans[0].parent().unwrap().to_path_buf();
    let mut h = harness(FakeDialogs::default());
    h.get_by_label("Restore").click();
    h.run_steps(1);
    drop_files(&mut h, &scans);
    wait(&mut h);
    assert_eq!(h.state().restore.target_dir(), Some(out.clone()));

    // Restore is impossible without the password.
    assert!(h.state().restore.needs_password());
    assert!(
        h.state()
            .restore
            .restore_blocker()
            .unwrap()
            .contains("password")
    );
    h.get_by_label("Backup password");

    // Wrong password → a clear error.
    h.state_mut().restore.password = "wrong".into();
    h.run_steps(1);
    h.get_by_label("Restore files").click();
    h.run_steps(1);
    wait(&mut h);
    assert!(
        matches!(&h.state().restore.restore, Task::Failed(e) if e.contains("Wrong password")),
        "{:?}",
        h.state().restore.restore
    );

    // Correct password → restored.
    h.state_mut().restore.password = "secret-password".into();
    h.run_steps(1);
    h.get_by_label("Restore files").click();
    h.run_steps(1);
    wait(&mut h);
    assert!(matches!(h.state().restore.restore, Task::Done(_)));
    assert!(out.join("family-photos/list.txt").exists());
}

/// Saves screenshots (for a visual check of the design).
/// Run: `PABARU_SCREENSHOTS=folder cargo test -p pabaru-gui -- --ignored screenshots`
#[test]
#[ignore]
fn screenshots() {
    let Ok(dir) = std::env::var("PABARU_SCREENSHOTS") else {
        return;
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A short fixed path, so the screenshots don't show a random temp folder.
    let root = if cfg!(unix) {
        PathBuf::from("/tmp/pabaru-demo")
    } else {
        std::env::temp_dir().join("pabaru-demo")
    };
    let _ = std::fs::remove_dir_all(&root);
    let (_, scans) = make_scans(&root, Some("password"));
    let folder = make_folder(&root.join("Documents"));

    for dark in [false, true] {
        let suffix = if dark { "dark" } else { "light" };
        let mut h = harness(FakeDialogs::default());
        h.ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        h.run_steps(3);
        save(&mut h, &dir.join(format!("1-start-{suffix}.png")));

        drop_files(&mut h, std::slice::from_ref(&folder));
        wait(&mut h);
        h.state_mut().backup.encrypt = true;
        h.state_mut().backup.password = "password".into();
        h.state_mut().backup.password_again = "password".into();
        h.state_mut().backup.recompute();
        h.run_steps(3);
        save(&mut h, &dir.join(format!("2-backup-{suffix}.png")));

        h.get_by_label("Restore").click();
        h.run_steps(2);
        save(&mut h, &dir.join(format!("3-restore-start-{suffix}.png")));
        drop_files(&mut h, &scans);
        wait(&mut h);
        save(&mut h, &dir.join(format!("4-restore-{suffix}.png")));
    }
    let _ = std::fs::remove_dir_all(&root);
}

fn save(h: &mut Harness<'static, PabaruApp>, path: &Path) {
    let img = h.render().expect("render");
    img.save(path).unwrap();
}
