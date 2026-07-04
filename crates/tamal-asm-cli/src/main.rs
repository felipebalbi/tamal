//! `tamal-asm` — command-line front-end: assemble source to bytecode, or
//! disassemble bytecode to a listing. Diagnostics render with ariadne.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ariadne::{Color, Label, Report, ReportKind, Source};
use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::eyre::{Context, Result};

use tamal_asm::{Diagnostic, assemble, disasm};

/// Assemble/disassemble tamal bytecode.
#[derive(Debug, Parser)]
#[command(name = "tamal-asm", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Assemble tamal source into bytecode.
    Assemble {
        /// Input `.s` source file.
        input: PathBuf,
        /// Output path (default: `<input>.bin` for bin; stdout for hex/listing).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Output format.
        #[arg(long, value_enum, default_value_t = Emit::Bin)]
        emit: Emit,
    },
    /// Disassemble bytecode into a textual listing.
    Disasm {
        /// Input bytecode (`.bin`) file.
        input: PathBuf,
        /// Output path (default: stdout).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Emit {
    /// Raw little-endian words (loader-ready).
    Bin,
    /// One 8-digit hex word per line.
    Hex,
    /// `addr  word  mnemonic ; source` table.
    Listing,
}

fn main() -> Result<ExitCode> {
    color_eyre::install()?;
    let cli = Cli::parse();
    match cli.command {
        Command::Assemble {
            input,
            output,
            emit,
        } => cmd_assemble(&input, output, emit),
        Command::Disasm { input, output } => cmd_disasm(&input, output),
    }
}

fn cmd_assemble(input: &Path, output: Option<PathBuf>, emit: Emit) -> Result<ExitCode> {
    let source =
        fs::read_to_string(input).wrap_err_with(|| format!("reading {}", input.display()))?;
    let name = input.display().to_string();
    let prog = match assemble(&source) {
        Ok(p) => p,
        Err(diags) => {
            for d in &diags {
                report(&name, &source, d);
            }
            return Ok(ExitCode::FAILURE);
        }
    };
    match emit {
        Emit::Bin => {
            let out = output.unwrap_or_else(|| input.with_extension("bin"));
            fs::write(&out, prog.to_le_bytes())
                .wrap_err_with(|| format!("writing {}", out.display()))?;
        }
        Emit::Hex => {
            let mut s = String::new();
            for w in prog.words() {
                s.push_str(&format!("{w:08x}\n"));
            }
            write_text(output, &s)?;
        }
        Emit::Listing => write_text(output, &prog.listing(&source))?,
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_disasm(input: &Path, output: Option<PathBuf>) -> Result<ExitCode> {
    let bytes = fs::read(input).wrap_err_with(|| format!("reading {}", input.display()))?;
    write_text(output, &disasm::disassemble(&bytes))?;
    Ok(ExitCode::SUCCESS)
}

fn write_text(output: Option<PathBuf>, text: &str) -> Result<()> {
    match output {
        Some(path) => {
            fs::write(&path, text).wrap_err_with(|| format!("writing {}", path.display()))?;
        }
        None => std::io::stdout()
            .write_all(text.as_bytes())
            .wrap_err("writing stdout")?,
    }
    Ok(())
}

fn report(name: &str, source: &str, d: &Diagnostic) {
    let mut b = Report::build(ReportKind::Error, (name, d.primary.clone()))
        .with_message(&d.message)
        .with_label(
            Label::new((name, d.primary.clone()))
                .with_message(&d.message)
                .with_color(Color::Red),
        );
    for (span, text) in &d.labels {
        b = b.with_label(
            Label::new((name, span.clone()))
                .with_message(text)
                .with_color(Color::Yellow),
        );
    }
    if let Some(help) = &d.help {
        b = b.with_help(help);
    }
    let _ = b.finish().eprint((name, Source::from(source)));
}
