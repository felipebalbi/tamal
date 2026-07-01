//! `tamal-asm` — command-line front-end for the tamal assembler.
//!
//! Placeholder entry point: the real subcommands (assemble a source file,
//! disassemble bytecode, dump the encoding) land in a later plan once
//! [`tamal_asm`] exposes a stable pipeline.

use clap::Parser;

/// Assemble RISC-V-flavored tamal source into tamal bytecode.
#[derive(Debug, Parser)]
#[command(name = "tamal-asm", version, about, long_about = None)]
struct Cli {}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let _cli = Cli::parse();
    println!("tamal-asm: scaffold — no commands implemented yet.");
    Ok(())
}
