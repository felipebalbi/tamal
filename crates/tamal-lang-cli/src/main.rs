//! `tamalc` — command-line front-end for the tamal-lang compiler. Compiles
//! `.tam` source to bytecode, generated assembly, or a listing. Front-end
//! diagnostics point at `.tam` source; the rare backend error points at the
//! generated assembly (a compiler bug). Rendering uses ariadne, mirroring
//! `tamal-asm-cli`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ariadne::{Color, Label, Report, ReportKind, Source};
use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::eyre::{Context, Result};

use tamal_lang::Diagnostic;

/// Compile tamal-lang source.
#[derive(Debug, Parser)]
#[command(name = "tamalc", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compile `.tam` source into bytecode, assembly, or a listing.
    Compile {
        /// Input `.tam` source file.
        input: PathBuf,
        /// Output path (default: `<input>.bin` for bin; stdout for asm/listing).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Output format.
        #[arg(long, value_enum, default_value_t = Emit::Bin)]
        emit: Emit,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Emit {
    /// Raw little-endian words (loader-ready).
    Bin,
    /// The generated tamal assembly (the trust bridge).
    Asm,
    /// `addr  word  mnemonic ; source` table over the generated assembly.
    Listing,
}

fn main() -> Result<ExitCode> {
    color_eyre::install()?;
    let cli = Cli::parse();
    match cli.command {
        Command::Compile {
            input,
            output,
            emit,
        } => cmd_compile(&input, output, emit),
    }
}

fn cmd_compile(input: &Path, output: Option<PathBuf>, emit: Emit) -> Result<ExitCode> {
    let source =
        fs::read_to_string(input).wrap_err_with(|| format!("reading {}", input.display()))?;
    let name = input.display().to_string();

    // Stage 1: lower to asm. Front-end diagnostics point at `.tam` source.
    let asm = match tamal_lang::lower_to_asm(&source) {
        Ok(a) => a,
        Err(diags) => {
            for d in &diags {
                report(&name, &source, d);
            }
            return Ok(ExitCode::FAILURE);
        }
    };

    if let Emit::Asm = emit {
        write_text(output, &asm)?;
        return Ok(ExitCode::SUCCESS);
    }

    // Stage 2: assemble. A backend error is a compiler bug; render it against
    // the generated assembly so it is still legible.
    let prog = match tamal_asm::assemble(&asm) {
        Ok(p) => p,
        Err(diags) => {
            let asm_name = format!("{name} (generated asm)");
            for d in &diags {
                report(&asm_name, &asm, d);
            }
            eprintln!("note: this error is in compiler-generated assembly — please report it");
            return Ok(ExitCode::FAILURE);
        }
    };

    match emit {
        Emit::Bin => {
            let out = output.unwrap_or_else(|| input.with_extension("bin"));
            fs::write(&out, prog.to_le_bytes())
                .wrap_err_with(|| format!("writing {}", out.display()))?;
        }
        Emit::Listing => write_text(output, &prog.listing(&asm))?,
        Emit::Asm => unreachable!("handled above"),
    }
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
