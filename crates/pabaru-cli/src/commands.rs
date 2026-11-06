//! Execution of the individual commands.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use pabaru_core::{
    BackupInfo, Crumple, Decoder, Destination, LayoutProposal, PageScan, Prepared, ScanSim, Tear,
    VerifyOptions, encode_prepared, load_images, prepare, suggest, verify,
};

use crate::args::{Command, CrumpleArg, TargetArgs, read_password};

pub fn run(cmd: Command) -> Result<()> {
    match cmd {
        Command::Encode {
            inputs,
            output,
            enc,
        } => {
            let prepared = prepare(&inputs, enc.level).context("packing the files failed")?;
            let opts = enc.to_options(&prepared)?;
            cmd_encode(&prepared, &output, &opts)
        }
        Command::Plan {
            inputs,
            target,
            encrypt,
            json,
            level,
        } => cmd_plan(&inputs, &target, encrypt, json, level),
        Command::Decode {
            scans,
            output,
            password_env,
        } => cmd_decode(&scans, output.as_deref(), &password_env),
        Command::Inspect { scan } => cmd_inspect(&scan),
        Command::Verify {
            inputs,
            enc,
            angle,
            blur,
            noise,
            scale,
            drop_pages,
            crumple,
            tear,
        } => {
            let vopts = VerifyOptions {
                sim: ScanSim {
                    angle_deg: angle,
                    blur_radius: blur,
                    noise,
                    scale,
                    crumple: crumple.map(|c| match c {
                        CrumpleArg::Medium => Crumple::MEDIUM,
                        CrumpleArg::Strong => Crumple::STRONG,
                    }),
                    tear: tear.then(Tear::default),
                    ..Default::default()
                },
                // The user numbers pages from 1, internally they start at 0.
                drop_pages: drop_pages.iter().map(|p| p.saturating_sub(1)).collect(),
            };
            let prepared = prepare(&inputs, enc.level).context("packing the files failed")?;
            cmd_verify(&inputs, &enc.to_options(&prepared)?, &vopts)
        }
    }
}

/// Human-readable size in bytes.
fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// Backup name for display. Names derived from the inputs are not stored
/// on paper (privacy), so the name is usually empty.
fn display_name(name: &str) -> String {
    if name.is_empty() {
        "(unnamed)".into()
    } else {
        format!("'{name}'")
    }
}

fn print_info(info: &BackupInfo) {
    println!(
        "Backup:      {} (ID {:08X})",
        display_name(&info.name),
        info.backup_id
    );
    println!(
        "Files:       {} ({}), compressed {}",
        info.file_count,
        human(info.raw_bytes),
        human(info.stream_len)
    );
    println!(
        "Blocks:      {} data + {} parity, {} per page",
        info.data_shards, info.parity_shards, info.cells_per_page
    );
    println!(
        "Pages:       {}{}{}",
        info.pages,
        if info.encrypted { ", encrypted" } else { "" },
        if info.survives_page_loss() {
            ", survives the loss of a whole page"
        } else {
            ""
        }
    );
}

fn cmd_encode(prepared: &Prepared, output: &Path, opts: &pabaru_core::EncodeOptions) -> Result<()> {
    let backup = encode_prepared(prepared, opts).context("encoding failed")?;
    print_info(&backup.info);
    let is_pdf = output
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    if is_pdf {
        backup.write_pdf(output)?;
        println!("Written:     {}", output.display());
    } else {
        let files = backup.write_pngs(output, "pabaru")?;
        println!(
            "Written:     {} PNG files to {}",
            files.len(),
            output.display()
        );
    }
    if backup.info.encrypted {
        println!(
            "Note: the password is not printed on paper. Without it the backup cannot be restored – keep it safe."
        );
    }
    let dpi = backup.params.dot_dpi().round() as u32;
    println!(
        "Tip: print at 100 % scale (no fit to page), scan at least {} dpi in grayscale.",
        dpi * 3
    );
    Ok(())
}

/// One row of the proposal table.
fn proposal_row(p: &LayoutProposal) -> String {
    format!(
        "{:<11} {:>6} {:>8} {:>8} {:>10} {:>7} {:>8.0} %  {}",
        p.profile.label(),
        p.pages,
        format!("{} dpi", p.dot_dpi),
        format!("{:.2} mm", p.dot_mm),
        format!("{} dpi", p.min_scan_dpi),
        p.parity_shards,
        p.loss_tolerance() * 100.0,
        if p.survives_page_loss() { "yes" } else { "no" }
    )
}

fn cmd_plan(
    inputs: &[PathBuf],
    target: &TargetArgs,
    encrypt: bool,
    json: bool,
    level: i32,
) -> Result<()> {
    let prepared = prepare(inputs, level).context("packing the files failed")?;
    let input = target.suggest_input(prepared.stream_len(encrypt))?;
    let proposals = suggest(&input);
    if json {
        println!("{}", serde_json::to_string_pretty(&proposals)?);
        return Ok(());
    }
    println!(
        "Data:        {} files, {} → compressed {}",
        prepared.file_count(),
        human(prepared.raw_bytes()),
        human(prepared.packed_len())
    );
    println!(
        "Devices:     {}, scanner {} dpi, paper {}",
        input.printer.label(),
        input.scanner_dpi,
        target.paper
    );
    if proposals.is_empty() {
        bail!(
            "no profile fits – try a scanner with a higher resolution, larger paper or a smaller backup"
        );
    }
    println!();
    // Column widths match `proposal_row`.
    println!("Profile      Pages     Dots      Dot  Min. scan  Parity Block loss  Page loss");
    for p in &proposals {
        println!("{}", proposal_row(p));
    }
    println!();
    for p in &proposals {
        println!("  {} – {}", p.profile.label(), p.profile.description());
    }
    println!();
    println!(
        "Usage: pabaru encode {} -o backup.pdf --profile balanced",
        inputs
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" ")
    );
    Ok(())
}

/// Text quality map: `.` no corrections, `1`–`9` number of corrections (per 3 bytes), `#` unreadable.
fn quality_map(scan: &PageScan) -> String {
    let mut out = String::new();
    for r in 0..scan.rows {
        for c in 0..scan.cols {
            let cell = &scan.cells[(r * scan.cols + c) as usize];
            out.push(match &cell.block {
                None => '#',
                Some(b) if b.corrected_bytes == 0 => '.',
                Some(b) => char::from(b'0' + (b.corrected_bytes.div_ceil(3)).min(9) as u8),
            });
        }
        out.push('\n');
    }
    out
}

fn cmd_decode(
    scans: &[PathBuf],
    output: Option<&Path>,
    password_env: &Option<String>,
) -> Result<()> {
    let mut dec = Decoder::new();
    for path in scans {
        let images = load_images(path).with_context(|| format!("loading {}", path.display()))?;
        let multi = images.len() > 1;
        for (n, img) in images.iter().enumerate() {
            let label = if multi {
                format!("{} [{}]", path.display(), n + 1)
            } else {
                path.display().to_string()
            };
            match dec.add_image(img) {
                Ok(r) => println!(
                    "{}: page {}, blocks {} OK / {} bad, new {}",
                    label,
                    r.page_index
                        .map(|p| (p + 1).to_string())
                        .unwrap_or_else(|| "?".into()),
                    r.ok_blocks,
                    r.failed_blocks,
                    r.new_shards
                ),
                Err(e) => eprintln!("{label}: {e}"),
            }
        }
    }

    let progress = dec
        .progress()
        .context("no Pabaru backup was found in the scans")?;
    println!(
        "Backup {}: read {} of the {} required blocks ({} total)",
        display_name(&progress.name),
        progress.have,
        progress.need,
        progress.total
    );
    if !progress.can_restore() {
        let missing: Vec<String> = progress
            .missing_pages()
            .iter()
            .map(|p| (p + 1).to_string())
            .collect();
        bail!(
            "not enough blocks to restore; pages not scanned: {}",
            if missing.is_empty() {
                "none (try a better scan)".into()
            } else {
                missing.join(", ")
            }
        );
    }

    let password = obtain_password(&dec, password_env)?;

    // Without -o, restore next to the scans (the folder of the first scan).
    let dir = match output {
        Some(dir) => dir.to_path_buf(),
        None => default_output_dir(scans),
    };
    println!("Restoring into {}", dir.display());
    let report = dec.restore(password.as_deref(), Destination::Folder(&dir))?;
    println!(
        "Restored and verified {} files ({} blocks reconstructed).",
        report.unpack.verified_files, report.recovered_shards
    );
    Ok(())
}

/// Default restore folder: the folder the first scan was loaded from.
fn default_output_dir(scans: &[PathBuf]) -> PathBuf {
    scans
        .first()
        .and_then(|p| p.parent())
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Number of password attempts in interactive mode.
const PASSWORD_ATTEMPTS: usize = 3;

/// Obtains the password if the backup is encrypted and verifies it right away
/// (by decrypting the stream). In interactive mode a typo can be retried
/// without having to load the scans again.
fn obtain_password(dec: &Decoder, password_env: &Option<String>) -> Result<Option<String>> {
    if !dec.needs_password() {
        return Ok(None);
    }
    println!("The backup is encrypted.");
    // Password from an environment variable: a single attempt (non-interactive scripts).
    let attempts = if password_env.is_some() {
        1
    } else {
        PASSWORD_ATTEMPTS
    };
    for attempt in 1..=attempts {
        let pw = read_password(password_env, false)?;
        match dec.check_password(&pw) {
            Ok(()) => return Ok(Some(pw)),
            Err(pabaru_core::CoreError::Crypto(_)) if attempt < attempts => {
                eprintln!(
                    "Wrong password, please try again ({} of {attempts}).",
                    attempt + 1
                );
            }
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!("the loop always ends with a return")
}

fn cmd_inspect(path: &Path) -> Result<()> {
    let images = load_images(path)?;
    let img = images.first().context("the file contains no page")?;
    let mut dec = Decoder::new();
    let r = dec.add_image(img)?;
    println!(
        "Grid {}×{} blocks, orientation {}, read {} / bad {}",
        r.scan.cols, r.scan.rows, r.scan.orientation, r.ok_blocks, r.failed_blocks
    );
    if let Some(sb) = dec.superblock() {
        println!(
            "Backup {} (ID {:08X}), page {}/{}, {} data + {} parity blocks{}",
            display_name(&sb.name),
            r.backup_id.unwrap_or_default(),
            sb.page_index + 1,
            sb.page_count,
            sb.data_shards,
            sb.parity_shards,
            if sb.flags.encrypted {
                ", encrypted"
            } else {
                ""
            }
        );
    }
    println!("Quality map (. no errors, 1–9 corrections, # unreadable):");
    print!("{}", quality_map(&r.scan));
    Ok(())
}

fn cmd_verify(
    inputs: &[PathBuf],
    opts: &pabaru_core::EncodeOptions,
    vopts: &VerifyOptions,
) -> Result<()> {
    println!("Simulating print and scan (this may take a while)…");
    let r = verify(inputs, opts, vopts).context("verification failed")?;
    print_info(&r.info);
    println!(
        "Scan:        {} pages, blocks {} OK / {} bad, {} reconstructed",
        r.pages_scanned, r.blocks_ok, r.blocks_failed, r.recovered_shards
    );
    println!(
        "Result:      OK – restored and verified {} files",
        r.verified_files
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(2048), "2.0 KiB");
        assert_eq!(human(5 * 1024 * 1024), "5.0 MiB");
    }

    #[test]
    fn empty_name_has_fallback() {
        assert_eq!(display_name(""), "(unnamed)");
        assert_eq!(display_name("photos"), "'photos'");
    }
}
