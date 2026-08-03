# tamal-lang for software engineers

> **Status:** designed, not yet implemented. This is a bridge from Python/C/Rust/
> JavaScript into `tamal-lang`. It leans on what you already know and is honest
> about what is deliberately missing.

`tamal-lang` is a small, real compiled language — lexer, parser, type checker,
const evaluator, module system, span-carrying diagnostics. If you have used
Rust, most of it will feel familiar. The surprises are all in one direction:
things you expect are *missing*, and each one is missing because the compile
target is a tiny FPGA engine with **16 registers, no stack, no data memory, and a
1024-instruction budget.** Understand that target and every constraint explains
itself.

---

## What is familiar

```rust
// comments like these
/* and these */

import espi                        // modules and imports
import "lib/foo.tam" as foo        // path imports with aliases

const PORT = 0x64                  // constants
enum Verdict: byte { Ok = 0x00, Crc = 0x11 }   // typed enums

fn iowr_hdr(op: byte, addr: int) -> bytes {    // pure functions
    [op, hi(addr), lo(addr)]
}

test io_read {                     // an entry point
    let hdr = iowr_hdr(op = 0x44, addr = PORT)   // named arguments, let-bindings
    // if / else, while, do/while below
}
```

- **Types:** `byte`, `int`, `bytes`, `bool`, `enum`, plus `reg` for runtime
  register values. `bytes` is the packet type — a compile-time byte-string with
  `++` concat, indexing, and `len()`.
- **Expressions:** the usual `+ - * / % & | ^ ~ << >>` and comparisons, plus
  builtins `crc8()`, `len()`, `lo()`, `hi()`. There is a full constant evaluator;
  think `const fn`, but for the whole expression language.
- **Modules:** one file = one module; items are private unless `pub`; references
  are qualified (`espi.command(...)`). Resolution is deterministic — canonical
  path dedupe, cycle detection, duplicate-different-body is an error.
- **Diagnostics:** `.tam` source spans, rendered with carets, like `rustc`.

Two callables, mapping to concepts you know:

| tamal-lang | Closest thing you know |
|---|---|
| `fn … -> T { expr }` | a `const fn` — pure, returns a compile-time value, no side effects |
| `proc … { … }` | a **macro that emits code** — it produces instructions and is *inlined* at each call site |

---

## What is deliberately missing (and why)

| You might reach for… | It is not here because… |
|---|---|
| runtime function calls / recursion | the ISA has **no `call`/`ret`/stack**. Every `proc` is inlined; there is no call frame to recurse into. |
| a heap, arrays, dynamic `bytes` at runtime | there is **no data memory**. `bytes` is a *compile-time* value; at runtime only the 16 registers exist. |
| unbounded `for`/`while` over data | `repeat N { … }` is a **compile-time unroll** (`N` constant). `while`/`do-while` exist for real runtime loops, but you are budgeting 1024 instructions. |
| `goto` / labels | withheld on purpose. The compiler owns branching so inlined procedures cannot collide labels. You get structured control flow; `--emit asm` shows the generated branches. |
| floats, strings | the wire is bytes. There are no floating-point or string types — `bytes` and `int` cover the domain. |
| files, sockets, threads, `now()`, `random()` | the compiler is **pure and deterministic**: identical source → byte-identical bytecode. Any impurity would break the rig's reproducibility, so none exists. |
| local mutable spill when you run out of registers | there is nowhere to spill. **More than 15 live variables is a compile error**, not a silent slowdown. Shorten a lifetime or `recv _` to discard. |

None of these is an oversight. The engine is a dumb SPI shifter with 16
registers; the language is a faithful, ergonomic front-end to *exactly that*, no
more.

---

## The mental shift: you are emitting a byte stream

The biggest adjustment is that a `test` is not a program that computes — it is a
program that **emits a precise sequence of wire bytes and reads the response.**
Most of your "logic" is compile-time (build the packet, fold the CRC); the
runtime part is small (shift bytes, poll, check a residue, halt with a verdict).

```rust
test io_read {
    config controller, x1, sck20, alert_pin
    frame {                                      // scope: CS# asserted here, deasserted on exit
        send [espi.PUT_IORD1, 0x00, 0x64] + crc8 // compile-time: bytes + folded CRC
        tar 2
        wait_state                               // runtime: poll the target
        recv data, status0, status1              // runtime: read the response
        expect crc else 0x11                     // runtime: check residue, else verdict
    }
    pass                                         // halt 0x00
}
```

`send`/`recv` are your I/O; `frame` is a scope guard (RAII — CS# always
deasserts); `pass`/`fail` are your return-with-status. The `+ crc8` is a
compile-time fold you never maintain by hand.

---

## `proc` vs `fn`, concretely

They inline differently because they *produce* different things:

```rust
fn hdr(addr: int) -> bytes { [0x44, hi(addr), lo(addr)] }   // returns a value

proc controller() { config controller, x1, sck20, alert_pin }   // emits an instruction
```

- `hdr(0x0064)` is **replaced by** `[0x44, 0x00, 0x64]` — a value, computed at
  compile time. It emits nothing on its own.
- `controller()` is **replaced by** the `set_config …` instruction it contains —
  it is a code template.

Because both inline with fresh registers, calling a `proc` never clobbers your
live variables. That is the whole reason register allocation can be safe without
a stack.

---

## Determinism is a hard contract

This is not a "nice for reproducible builds" aspiration; it is load-bearing for a
compliance rig. Identical source produces identical bytecode, always. The
constant evaluator touches no clock, environment, filesystem order, or RNG; every
output-bearing collection is ordered; artifacts carry no timestamps or absolute
paths. When a conformance run fails, you can hand someone the `.tam` and they get
the same bytes and the same failure.

The one place randomness will *ever* appear — the deferred compile-time
error-injection feature — is specified to stay deterministic too: the ratio is
rational so `ratio = 0` is *exactly* zero, and a given `(seed, ratio)` replays
byte-for-byte. There is no run-time RNG anywhere in the design.

---

## Where to go next

- Write your first test in five minutes → [getting-started.md](getting-started.md).
- The complete construct-by-construct guide → [language-guide.md](language-guide.md).
- The `espi` library you will import constantly → [stdlib-espi.md](stdlib-espi.md).
- Recipes to copy → [cookbook.md](cookbook.md).
