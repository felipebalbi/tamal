//! `tamal-loader` — command-line loader and controller for the eSPI rig.
//!
//! Placeholder entry point: the real subcommands (load a compiled program,
//! set role/IO-mode/CRC/injection, arm, stream results/verdicts) land in a
//! later plan once `tamal-loader` exposes the device API.

use clap::Parser;

/// Load tamal bytecode onto a rig and control it.
#[derive(Debug, Parser)]
#[command(name = "tamal-loader", version, about, long_about = None)]
struct Cli {}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let _cli = Cli::parse();
    println!("tamal-loader: scaffold — no commands implemented yet.");
    Ok(())
}
