//! `tamal-loader` — load a compiled program onto a rig, run it, and print the
//! drained trace with a HALT/TRAP verdict.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Context, Result};

use tamal_abi::trace::{Halt, Record, Revision, Trace, TrapReason};
use tamal_loader::transport::UartTransport;
use tamal_loader::{Device, RunOptions, validate_program_bytes};

/// Load tamal bytecode onto a rig, run it, and print the drained trace.
#[derive(Debug, Parser)]
#[command(name = "tamal-loader", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Load a `.bin`, trigger a run, and print the drained trace + verdict.
    Run {
        /// The compiled program (`tamal-asm` `.bin`).
        program: PathBuf,
        /// The serial port (e.g. `/dev/tty.usbserial-XXXX`).
        #[arg(short, long)]
        port: String,
        /// UART baud rate.
        #[arg(long, default_value_t = 2_000_000)]
        baud: u32,
        /// Per-drain read timeout, in seconds.
        #[arg(long, default_value_t = 5)]
        timeout: u64,
        /// Extra attempts after the first on a lost/garbled drain.
        #[arg(long, default_value_t = 3)]
        retries: u32,
    },
}

fn reason_str(r: TrapReason) -> &'static str {
    match r {
        TrapReason::None => "none",
        TrapReason::Decode => "decode",
        TrapReason::Config => "config",
        TrapReason::Rdsr => "rdsr",
        TrapReason::Illegal => "illegal",
    }
}

fn format_halt(h: &Halt) -> String {
    if h.trap {
        format!(
            "TRAP  reason={}  ovf={}  status={:#04X}",
            reason_str(h.reason),
            h.ovf,
            h.status
        )
    } else if h.ovf {
        format!(
            "HALT  status={:#04X}  ovf=true  (trace overflow — records dropped)",
            h.status
        )
    } else {
        format!("HALT  status={:#04X}  (ok)", h.status)
    }
}

fn format_trace(t: &Trace) -> String {
    let r = &t.revision;
    let mut s = format!("REVISION {}.{}.{}\n", r.major, r.minor, r.patch);
    for (i, rec) in t.records.iter().enumerate() {
        match rec {
            Record::Capture { nbits, byte } => {
                s.push_str(&format!("[{i}] CAPTURE  nbits={nbits}  byte={byte:#04X}\n"));
            }
            Record::Mark { label, payload } => {
                s.push_str(&format!(
                    "[{i}] MARK     label={label:#06X}  payload={payload:#010X}\n"
                ));
            }
        }
    }
    s.push_str(&format_halt(&t.halt));
    s.push('\n');
    s
}

fn trace_exit_code(h: &Halt) -> u8 {
    if h.trap || h.ovf { 1 } else { 0 }
}

fn cmd_run(
    program: PathBuf,
    port: String,
    baud: u32,
    timeout: u64,
    retries: u32,
) -> Result<ExitCode> {
    let bytes = fs::read(&program).wrap_err_with(|| format!("reading {}", program.display()))?;
    let words = validate_program_bytes(&bytes)?;
    let transport = UartTransport::open(&port, baud)?;
    let mut device = Device::new(transport);
    let opts = RunOptions {
        timeout: Duration::from_secs(timeout),
        retries,
    };
    let trace = device.run(&words, opts)?;
    if trace.revision != Revision::EXPECTED {
        eprintln!(
            "warning: gateware revision {}.{}.{} != expected {}.{}.{} (bitstream/CLI mismatch)",
            trace.revision.major,
            trace.revision.minor,
            trace.revision.patch,
            Revision::EXPECTED.major,
            Revision::EXPECTED.minor,
            Revision::EXPECTED.patch,
        );
    }
    print!("{}", format_trace(&trace));
    Ok(ExitCode::from(trace_exit_code(&trace.halt)))
}

fn main() -> Result<ExitCode> {
    color_eyre::install()?;
    let cli = Cli::parse();
    match cli.command {
        Command::Run {
            program,
            port,
            baud,
            timeout,
            retries,
        } => cmd_run(program, port, baud, timeout, retries),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Trace {
        Trace {
            revision: Revision::EXPECTED,
            records: vec![
                Record::Capture {
                    nbits: 8,
                    byte: 0x5A,
                },
                Record::Mark {
                    label: 1,
                    payload: 0xDEAD_BEEF,
                },
            ],
            halt: Halt {
                trap: false,
                reason: TrapReason::None,
                ovf: false,
                status: 0,
            },
        }
    }

    #[test]
    fn formats_records_and_ok_verdict() {
        let s = format_trace(&sample());
        assert!(s.contains("REVISION 0.1.0"), "{s}");
        assert!(s.contains("CAPTURE") && s.contains("byte=0x5A"), "{s}");
        assert!(
            s.contains("MARK") && s.contains("payload=0xDEADBEEF"),
            "{s}"
        );
        assert!(s.contains("HALT  status=0x00  (ok)"), "{s}");
    }

    #[test]
    fn formats_trap_verdict() {
        let h = Halt {
            trap: true,
            reason: TrapReason::Decode,
            ovf: false,
            status: 0x11,
        };
        assert!(format_halt(&h).contains("TRAP  reason=decode"));
        assert_eq!(trace_exit_code(&h), 1);
        assert_eq!(
            trace_exit_code(&Halt {
                trap: false,
                reason: TrapReason::None,
                ovf: false,
                status: 0
            }),
            0
        );
    }

    #[test]
    fn clean_halt_with_overflow_is_surfaced_and_nonzero_exit() {
        let h = Halt {
            trap: false,
            reason: TrapReason::None,
            ovf: true,
            status: 0,
        };
        let s = format_halt(&h);
        assert!(s.contains("ovf=true") && s.contains("overflow"), "{s}");
        assert!(!s.contains("(ok)"), "overflow must not read as ok: {s}");
        assert_eq!(trace_exit_code(&h), 1);
    }
}
