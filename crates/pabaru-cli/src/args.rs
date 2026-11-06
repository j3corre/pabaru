//! Command-line argument definitions (clap) and their conversion into settings.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use pabaru_core::{
    EncodeOptions, Paper, Prepared, PrintParams, Printer, Profile, SuggestInput, propose,
};

/// Pabaru – back up files and folders to paper.
#[derive(Debug, Parser)]
#[command(name = "pabaru", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Back up files and folders into a PDF or PNGs for printing.
    Encode {
        /// Files and folders to back up (more than one may be given).
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Output: a .pdf file, or a folder for PNG pages.
        #[arg(short, long)]
        output: PathBuf,
        #[command(flatten)]
        enc: EncodeArgs,
    },
    /// Propose optimal backup parameters (page count, density, resilience).
    Plan {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[command(flatten)]
        target: TargetArgs,
        /// Account for encryption (adds a few bytes).
        #[arg(long)]
        encrypt: bool,
        /// Output in JSON format (for scripts).
        #[arg(long)]
        json: bool,
        /// zstd compression level (1–22).
        #[arg(long, default_value_t = 19)]
        level: i32,
    },
    /// Restore files from page scans (PNG, JPEG, TIFF, BMP, PDF).
    Decode {
        /// Scanned pages in any order.
        #[arg(required = true)]
        scans: Vec<PathBuf>,
        /// Target folder for restored files (default: the folder of the first scan).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Name of the environment variable holding the password (otherwise you are prompted).
        #[arg(long)]
        password_env: Option<String>,
    },
    /// Print information about a scanned page and a block quality map.
    Inspect {
        /// Scanned page.
        scan: PathBuf,
    },
    /// Verify a backup without a printer: simulated print and scan, restore, hash check.
    Verify {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[command(flatten)]
        enc: EncodeArgs,
        /// Rotation of the simulated scan in degrees.
        #[arg(long, default_value_t = 1.5)]
        angle: f64,
        /// Blur (radius in pixels).
        #[arg(long, default_value_t = 1)]
        blur: u32,
        /// Noise (± brightness).
        #[arg(long, default_value_t = 15)]
        noise: u8,
        /// Ratio of scan to print resolution (1.0 = equal).
        #[arg(long, default_value_t = 1.0)]
        scale: f64,
        /// Simulate the loss of a page (numbered from 1, may be repeated).
        #[arg(long = "drop-page")]
        drop_pages: Vec<usize>,
        /// Simulate crumpled paper.
        #[arg(long, value_enum)]
        crumple: Option<CrumpleArg>,
        /// Simulate tearing each page into two pieces.
        #[arg(long)]
        tear: bool,
    },
}

/// Degree of simulated crumpling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CrumpleArg {
    /// Page crumpled in the hand and smoothed out.
    Medium,
    /// Tightly crumpled ball.
    Strong,
}

/// Printer type (for parameter proposals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PrinterArg {
    Laser600,
    Laser1200,
    Inkjet,
}

impl From<PrinterArg> for Printer {
    fn from(p: PrinterArg) -> Self {
        match p {
            PrinterArg::Laser600 => Printer::Laser600,
            PrinterArg::Laser1200 => Printer::Laser1200,
            PrinterArg::Inkjet => Printer::Inkjet,
        }
    }
}

/// Resilience profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ProfileArg {
    /// Least paper.
    Compact,
    /// A trade-off.
    Balanced,
    /// Survives the loss of a whole page.
    Archival,
}

impl From<ProfileArg> for Profile {
    fn from(p: ProfileArg) -> Self {
        match p {
            ProfileArg::Compact => Profile::Compact,
            ProfileArg::Balanced => Profile::Balanced,
            ProfileArg::Archival => Profile::Archival,
        }
    }
}

/// Target devices: paper, printer, scanner.
#[derive(Debug, Args, Clone)]
pub struct TargetArgs {
    /// Paper: a4, letter or WIDTHxHEIGHT in mm (e.g. 210x297).
    #[arg(long, default_value = "a4")]
    pub paper: String,
    /// Paper margin in mm.
    #[arg(long, default_value_t = 10.0)]
    pub margin: f32,
    /// Printer type (for --profile and plan).
    #[arg(long, value_enum, default_value_t = PrinterArg::Laser600)]
    pub printer: PrinterArg,
    /// Optical resolution (dpi) of the scanner that will be used for restoring.
    #[arg(long, default_value_t = 600)]
    pub scanner_dpi: u32,
}

impl TargetArgs {
    /// Optimizer input for a stream of the given length.
    pub fn suggest_input(&self, stream_len: u64) -> Result<SuggestInput> {
        Ok(SuggestInput {
            stream_len,
            printer: self.printer.into(),
            scanner_dpi: self.scanner_dpi,
            paper: parse_paper(&self.paper)?,
            margin_mm: self.margin,
        })
    }
}

/// Shared encoding settings.
#[derive(Debug, Args, Clone)]
pub struct EncodeArgs {
    #[command(flatten)]
    pub target: TargetArgs,
    /// Use a proposed profile (overrides --dot-dpi, --redundancy, --fill
    /// and --survive-page-loss according to the optimizer).
    #[arg(long, value_enum)]
    pub profile: Option<ProfileArg>,
    /// Physical printer resolution (dpi) for manual settings.
    #[arg(long, default_value_t = 600)]
    pub printer_dpi: u32,
    /// Dot density (dpi); must divide the printer resolution evenly.
    #[arg(long, default_value_t = 200)]
    pub dot_dpi: u32,
    /// Dot size in % of the pitch.
    #[arg(long, default_value_t = 70)]
    pub fill: u8,
    /// Minimum redundancy in % (parity blocks relative to data blocks).
    #[arg(long, default_value_t = 20)]
    pub redundancy: u32,
    /// Guarantee recovery even after losing a whole page.
    #[arg(long)]
    pub survive_page_loss: bool,
    /// Encrypt with a password (you will be prompted for it).
    #[arg(long)]
    pub encrypt: bool,
    /// Name of the environment variable holding the password (enables encryption, for scripts).
    #[arg(long)]
    pub password_env: Option<String>,
    /// Optional backup label stored on paper (default: none; input names are never stored).
    #[arg(long, default_value = "")]
    pub name: String,
    /// zstd compression level (1–22).
    #[arg(long, default_value_t = 19)]
    pub level: i32,
}

/// Parses a paper specification into [`Paper`].
pub fn parse_paper(s: &str) -> Result<Paper> {
    match s.to_ascii_lowercase().as_str() {
        "a4" => Ok(Paper::A4),
        "letter" => Ok(Paper::Letter),
        other => {
            let (w, h) = other
                .split_once('x')
                .context("paper must be a4, letter or WIDTHxHEIGHT in mm")?;
            Ok(Paper::Custom {
                width_mm: w.trim().parse().context("invalid paper width")?,
                height_mm: h.trim().parse().context("invalid paper height")?,
            })
        }
    }
}

/// Reads the password from an environment variable or interactively.
pub fn read_password(env: &Option<String>, confirm: bool) -> Result<String> {
    if let Some(var) = env {
        return std::env::var(var).with_context(|| format!("variable {var} is not set"));
    }
    let pw = rpassword::prompt_password("Password: ")?;
    if confirm {
        let again = rpassword::prompt_password("Password again: ")?;
        if pw != again {
            bail!("passwords do not match");
        }
    }
    if pw.is_empty() {
        bail!("password must not be empty");
    }
    Ok(pw)
}

impl EncodeArgs {
    /// Builds the settings for `pabaru-core`. `prepared` (the packed data)
    /// is needed to propose parameters for `--profile`.
    pub fn to_options(&self, prepared: &Prepared) -> Result<EncodeOptions> {
        let encrypted = self.encrypt || self.password_env.is_some();
        if let Some(profile) = self.profile {
            let input = self.target.suggest_input(prepared.stream_len(encrypted))?;
            let proposal = propose(&input, profile.into()).context(
                "no suitable parameters found for the chosen profile (weak scanner, small paper, or backup too large)",
            )?;
            let password = if encrypted {
                Some(read_password(&self.password_env, true)?)
            } else {
                None
            };
            return Ok(EncodeOptions {
                password,
                zstd_level: self.level,
                name: self.name.clone(),
                ..EncodeOptions::from_proposal(&proposal)
            });
        }
        if self.dot_dpi == 0 || !self.printer_dpi.is_multiple_of(self.dot_dpi) {
            bail!(
                "dot density {} dpi must divide the printer resolution {} dpi evenly",
                self.dot_dpi,
                self.printer_dpi
            );
        }
        if !(30..=100).contains(&self.fill) {
            bail!("dot size must be 30–100 %");
        }
        let password = if encrypted {
            Some(read_password(&self.password_env, true)?)
        } else {
            None
        };
        Ok(EncodeOptions {
            params: PrintParams {
                printer_dpi: self.printer_dpi,
                px_per_dot: self.printer_dpi / self.dot_dpi,
                dot_fill_pct: self.fill,
                paper: parse_paper(&self.target.paper)?,
                margin_mm: self.target.margin,
                ..Default::default()
            },
            redundancy_pct: self.redundancy,
            survive_page_loss: self.survive_page_loss,
            password,
            zstd_level: self.level,
            name: self.name.clone(),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper() {
        assert_eq!(parse_paper("A4").unwrap(), Paper::A4);
        assert_eq!(
            parse_paper("100x150").unwrap(),
            Paper::Custom {
                width_mm: 100.0,
                height_mm: 150.0
            }
        );
        assert!(parse_paper("abc").is_err());
    }

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
