# tamal-loader-cli

The command-line loader and controller for a tamal eSPI rig. Ships the
**`tamal-loader`** binary: load a compiled program, trigger a run, drain the
trace, and print it with a HALT/TRAP verdict.

Part of the [tamal](../../README.md) eSPI compliance rig. **MIT-licensed.** Wraps
the [`tamal-loader`](../tamal-loader) library.

## Usage

```sh
tamal-loader run <PROGRAM.bin> --port <PORT> [--baud 2000000] [--timeout 5] [--retries 3]
```

| Flag | Default | Meaning |
|------|---------|---------|
| `<PROGRAM>` | — | the compiled `.bin` from `tamal-asm assemble` |
| `--port` | *(required)* | serial port, e.g. `/dev/tty.usbserial-XXXX` |
| `--baud` | `2000000` | UART baud (matches the gateware's 2 MBaud top) |
| `--timeout` | `5` | per-drain read timeout, seconds |
| `--retries` | `3` | extra attempts after the first on a lost/garbled drain |

Example output:

```
REVISION 0.1.0
[0] CAPTURE  nbits=8  byte=0x5A
[1] MARK     label=0x0001  payload=0xDEADBEEF
HALT  status=0x00  (ok)
```

**Exit code:** `0` on a clean HALT; **non-zero** on a `TRAP` *or* a trace-ring
overflow (records dropped) — so scripts can gate on the verdict. A drained
`REVISION` other than the CLI's expected version prints a warning to stderr
(bitstream/CLI mismatch).

## First silicon smoke test (no eSPI DUT required)

A one-instruction program that just halts exercises the whole
host → UART → loader → engine → drain path:

```sh
printf 'halt 0\n' > halt.s
tamal-asm assemble halt.s -o halt.bin
tamal-loader run halt.bin --port /dev/tty.usbserial-XXXX
# expect: REVISION 0.1.0  /  HALT  status=0x00  (ok)  /  exit 0
```

## Where it sits

```
tamal-loader-cli ──► tamal-loader ──► tamal-abi + serialport
                 └──► tamal-abi
```

## Build / run

```sh
cargo run -p tamal-loader-cli -- run --help
cargo install --path crates/tamal-loader-cli   # installs the `tamal-loader` binary
```

## See also

- Library: [`tamal-loader`](../tamal-loader)
- Host-loader design: [`docs/superpowers/specs/2026-07-04-tamal-loader-host-design.md`](../../docs/superpowers/specs/2026-07-04-tamal-loader-host-design.md)
