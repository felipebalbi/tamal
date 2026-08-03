# The tamal-lang language guide

> **Status:** designed, not yet implemented. This guide documents the approved
> design in
> [`docs/superpowers/specs/2026-07-20-tamal-lang-design.md`](../superpowers/specs/2026-07-20-tamal-lang-design.md).
> Assembly shown under *"lowers to"* is the intended output of `tamalc`; it is
> real tamal-asm that the `tamal-asm` backend already accepts.

This is the long-form teaching reference. Each section answers four questions:
**what** the construct is, **how** you use it, **what it lowers to**, and **what
trips people up**. Read it front to back once; use it as a lookup afterward.

A recurring theme: `tamal-lang` folds *boilerplate*, never *meaning*. The wire
bytes are always visible in `--emit-asm`, the CRC is always the same code that
runs on the wire, and deliberate rule-breaking is always loud. Keep that in mind
and the design choices explain themselves.

**Contents**

1. [Lexical basics: comments, numbers, literals](#1-lexical-basics)
2. [The type system](#2-the-type-system)
3. [`const` and `enum`](#3-const-and-enum)
4. [Expressions and compile-time evaluation](#4-expressions-and-compile-time-evaluation)
5. [`crc8` — the three CRC surfaces](#5-crc8--the-three-crc-surfaces)
6. [Deliberate-wrong: bad CRCs and illegal turnarounds](#6-deliberate-wrong)
7. [`proc` and `fn`](#7-proc-and-fn)
8. [Structured control flow](#8-structured-control-flow)
9. [The named-variable register model](#9-the-named-variable-register-model)
10. [Domain sugar: `config`, `frame`, `send`, `recv`, `wait_state`, `expect`, `pass`, `fail`, `mark`](#10-domain-sugar)
11. [`test`](#11-test)
12. [Imports and modules](#12-imports-and-modules)
13. [A note on read accounting](#a-note-on-read-accounting)

---

## 1. Lexical basics

### Comments

```rust
// line comment — everything to end of line
/* block comment —
   spans multiple lines */
```

Same as Rust, C, JavaScript. Block comments do not nest.

### Numbers

Numbers are compile-time integers. Four notations, an optional sign, and `_`
digit separators anywhere between digits:

```rust
100          // decimal
0x64         // hexadecimal
0b0110_0100  // binary
0xDE_AD      // underscores group digits; ignored by the compiler
-1           // negative
```

`_` is purely visual — `0xDE_AD` and `0xDEAD` are the same number. Use it to
group nibbles, bytes, or bit-fields for readability.

Every number is range-checked against **where it is used**, not at the literal.
`0x64` is fine as a byte; `0x1_0000` is fine as an `int` but an error the moment
you try to send it as one wire byte. Out-of-range is always a compile error,
never a silent truncation or wrap.

### Byte and packet literals

A **packet literal** is a comma-separated list of byte values in square
brackets. Its type is `bytes` (see [§2](#2-the-type-system)).

```rust
[0x00, 0x64, 0xAB]        // three bytes
[espi.PUT_IORD1, 0x00]    // elements can be any compile-time byte expression
[]                        // the empty packet is legal
```

Concatenate packets with `++`:

```rust
[0x06, 0x21] ++ [0x00, 0x04]      // => [0x06, 0x21, 0x00, 0x04]
header ++ payload ++ [crc8(...)]  // build a frame in pieces
```

`++` is compile-time: it produces a new `bytes` value; nothing is emitted until
you `send` it.

---

## 2. The type system

Six value types. They exist so operand positions type-check — you cannot, for
example, feed a runtime register to the compile-time CRC.

| Type | What it is | Example | Notes |
|---|---|---|---|
| `byte` | one wire byte, `0..=255` | `0x64` | narrower than `int`; the unit of the bus |
| `int` | a compile-time integer | `0x0064`, `-3` | must narrow to fit its target field |
| `bytes` | a compile-time byte-string (the packet type) | `[0x44, 0x00]` | concat with `++`, index, `len` |
| `bool` | a condition value | `resp == 0x0F` | used by `if`/`while` |
| `reg` | a **runtime** register variable | a `recv` target | the value is only known on the rig, not at compile time |
| `enum` | a named set of `byte` values | `Verdict.Crc` | see [§3](#3-const-and-enum) |

The load-bearing distinction is **compile-time (`byte`/`int`/`bytes`/`bool`/
`enum`) versus runtime (`reg`)**:

- Compile-time values are folded away — they become immediates in the assembly.
- `reg` values are only known when the program runs on the rig. They come from
  the bus (`recv`, `get_byte`), and they can be used where the engine expects a
  register: as a `put_byte` source, a `mark` payload, a `tar` width, or a
  branch condition.
- **`crc8()` of a `reg` is a type error** — the CRC is compile-time only. This
  is a *feature*: it means every CRC the compiler folds is over bytes it can
  see, so the folded byte can never disagree with the wire.

---

## 3. `const` and `enum`

### `const`

A `const` binds a name to a compile-time value. It never occupies a register; it
folds to an immediate wherever it is used.

```rust
const PUT_IORD1 = 0x44
const PORT_8042 = 0x64
const HEADER    = [PUT_IORD1, 0x00, PORT_8042]   // a bytes const is fine too
```

Used as an operand, a `const` is indistinguishable from writing the literal:

```rust
send [PUT_IORD1, 0x00, PORT_8042] + crc8
```

lowers to exactly the same `put_byte 0x44 / put_byte 0x00 / put_byte 0x64 …` as
if you had typed the numbers. This is the tamal-lang replacement for the
assembler's `.equ`.

### `enum`

An `enum` is a named set of `byte` values with an explicit width. It is how the
`espi` library gives names to verdict codes, cycle types, and response codes.

```rust
enum Verdict: byte {
    Ok  = 0x00,
    Crc = 0x11,
}
```

- `: byte` is the backing type; every member must fit in it.
- Refer to a member with dot syntax: `Verdict.Crc` is the `byte` `0x11`.
- Members fold like constants:

```rust
fail Verdict.Crc     // lowers to  halt 0x11
```

Enums do not add runtime cost. They are documentation that type-checks.

---

## 4. Expressions and compile-time evaluation

`tamal-lang` has a pure, total **constant evaluator**. It runs at compile time,
reads no clock, environment, filesystem order, or RNG, and therefore turns
identical source into byte-identical bytecode on every machine. That determinism
is not a nicety — it is what makes the rig reproducible.

Operators, roughly C-flavored:

| Category | Operators |
|---|---|
| arithmetic | `+  -  *  /  %` |
| bitwise | `&  \|  ^  ~  <<  >>` |
| comparison | `==  !=  <  <=  >  >=` |
| `bytes` | `++` (concat), indexing, `len(b)` |

Builtins usable in any compile-time expression:

| Builtin | Signature | Meaning |
|---|---|---|
| `crc8(b)` | `bytes -> byte` | CRC-8 (poly `0x07`, init `0x00`, MSB-first) over `b` |
| `len(b)` | `bytes -> int` | number of bytes in `b` |
| `lo(n)` | `int -> byte` | low byte, `n & 0xFF` |
| `hi(n)` | `int -> byte` | next byte up, `(n >> 8) & 0xFF` |

Worked example — build a big-endian I/O header from an address:

```rust
const ADDR = 0x0064
[PUT_IORD1, hi(ADDR), lo(ADDR)]    // => [0x44, 0x00, 0x64]
```

`hi(0x0064)` is `0x00`, `lo(0x0064)` is `0x64`, so the packet is `[0x44, 0x00,
0x64]` — the exact command bytes of the peripheral I/O read. Everything here
happens at compile time; the result is three `put_byte` immediates.

**Pitfall — narrowing.** Arithmetic happens in `int`, but the *result* must fit
where you put it. `0x80 + 0x80` is `0x100`, a fine `int`; sent as one byte it is
a compile error ("byte out of range"). This is deliberate: the assembler would
also reject it, and the language catches it earlier with a better message.

---

## 5. `crc8` — the three CRC surfaces

CRC-8 is the headline reliability feature, so it gets three spellings for three
situations. **All three fold over exactly the bytes that are emitted, in
emission order.** The compiler literally calls `tamal_abi::crc8`, the same
function that runs on the wire and is mirrored byte-for-byte in the HDL
`Tamal.Crc`. There is no second transcription to drift.

### Surface 1 — `crc8(pkt)` as a value

`crc8(pkt)` with an argument is an ordinary compile-time expression that yields a
`byte`. Use it anywhere a byte is expected.

```rust
const CMD = [0x44, 0x00, 0x64]
put_byte crc8(CMD)          // lowers to:  put_byte 0x16
```

Because it is just a value, you can transform it — which is how deliberate-wrong
tests are written (see [§6](#6-deliberate-wrong)).

### Surface 2 — `send pkt + crc8` (the append sugar)

The most common case: send a packet and append its CRC. The bare keyword `crc8`
(no arguments) after `+` means *"the CRC of the bytes this `send` just emitted."*

```rust
send [0x44, 0x00, 0x64] + crc8
```

lowers to:

```asm
        put_byte 0x44
        put_byte 0x00
        put_byte 0x64
        put_byte 0x16          # + crc8  = crc8([0x44,0x00,0x64])
```

Edit any command byte and the appended CRC recomputes on the next build. This is
the line that replaces the single most error-prone statement in every
hand-written `.s` (`put_byte 0x16   # TX CRC-8`, hand-computed).

> **Do not confuse the two spellings.** `+ crc8` (a bare keyword, no parentheses)
> is the append sugar over the current `send`. `crc8(pkt)` (with parentheses and
> an argument) is the value builtin. The first is structural; the second is an
> expression you can manipulate.

### Surface 3 — `crc_region { … }` (structural block)

When the covered bytes are built across several statements — or with control
flow — wrap them in `crc_region { … }`. The CRC is folded over **exactly the
bytes emitted inside the block**, then appended.

```rust
crc_region {
    send [espi.PUT_OOB, 0x21, 0x00, 0x04]     // eSPI OOB header
    send [0x10, 0x00, 0x01, 0xAB]             // tunneled SMBus payload
}
```

lowers to:

```asm
        put_byte 0x06          # PUT_OOB
        put_byte 0x21
        put_byte 0x00
        put_byte 0x04
        put_byte 0x10
        put_byte 0x00
        put_byte 0x01
        put_byte 0xAB
        put_byte 0xB1          # crc_region: crc8 over the 8 bytes just emitted
```

The guarantee is **structural**: the bytes the CRC covers and the bytes actually
emitted are the same bytes, by construction. You cannot accidentally CRC a
different set than you sent, because there is only one set.

### Why this matters

All three surfaces share one CRC implementation with the wire and the HDL, and
all three bind the covered bytes to the emitted bytes. The stale-hand-CRC hazard
— edit a payload byte, forget to recompute the CRC, ship a test that silently
passes bad packets — cannot happen. That single class of bug is what motivated
the language.

---

## 6. Deliberate-wrong

A compliance rig exists to send **illegal** cycles on purpose: a corrupted CRC to
confirm the DUT rejects it, an illegal turnaround to confirm the DUT recovers.
The rule is: an ergonomic abstraction must never *silently* legalize an intended
violation, and the intent must be **loud and greppable**.

### A deliberately wrong CRC

Never write a bad CRC as a bare literal — a future reader cannot tell it from a
stale mistake, and a linter cannot tell either. Instead, derive it from the
correct CRC with a visible corruption:

```rust
// deliberately corrupt the CRC to confirm the DUT rejects the packet
send pkt ++ [crc8(pkt) ^ 0xFF]     // the ^0xFF is the signal of intent
```

For `pkt = [0x44, 0x00, 0x64]`, `crc8(pkt)` is `0x16`, so `0x16 ^ 0xFF` is
`0xE9`, and this lowers to:

```asm
        put_byte 0x44
        put_byte 0x00
        put_byte 0x64
        put_byte 0xE9          # crc8(pkt) ^ 0xFF — deliberately wrong
```

Note the two differences from the correct form:

- It uses `++ [ … ]` (explicit `bytes` concat), **not** `+ crc8` (the
  correct-by-construction append). The syntax itself signals "I am appending a
  hand-controlled byte."
- The `^ 0xFF` is greppable. Searching the tree for `crc8(` `^` finds every
  intentional corruption.

The optional `--lint` pass recomputes `crc8()` over each covered payload and
flags any literal CRC byte that disagrees — *unless* it is marked
deliberate-wrong like this. So an accidental stale byte is caught; an intentional
one is left alone.

### An illegal turnaround

Turnaround width is **always an explicit number**, never hidden inside a helper.
The legal turnaround for these transfers is `tar 2`; a violation is simply a
different width:

```rust
tar 3      // deliberate TAR violation — an explicit, visible width
```

which lowers, unchanged, to `tar 3`. The `tar` encoding accepts `0..15`, so the
*assembler* will not stop you (the width is a legal field value); the *eSPI
protocol* is what `tar 3` violates. That is exactly the point: the language
drives the illegal cycle you asked for and hides nothing. There is no
"turnaround" helper that could quietly normalize `3` back to `2`.

---

## 7. `proc` and `fn`

There are two kinds of callable, and the difference is the whole game.

### The constraint that shapes both

The tamal ISA has **no `call`, no `ret`, no stack, no data memory.** There is no
runtime subroutine linkage to invent. So neither callable is a runtime call:

- **`proc`** — a procedure that *emits instructions*. It is **inlined** at every
  call site.
- **`fn`** — a pure function that *returns a value* (`byte`, `int`, or `bytes`).
  Its call is **replaced by the returned constant** at compile time.

Both fully disappear before assembly. This is stated up front because it is why
register hygiene ([§9](#9-the-named-variable-register-model)) can be guaranteed.

### `proc` — emitting, inlined

```rust
proc controller() {
    config controller, x1, sck20, alert_pin
}
```

A call `controller()` inlines the body:

```asm
        set_config controller, x1, sck20, alert_pin
```

Procedures capture the emit boilerplate — the framing, the poll, the verdict —
once, in one reviewed place. The bundled `espi.command` proc
([stdlib-espi.md](stdlib-espi.md)) is the canonical example: the poll/residue/
verdict skeleton that was copy-pasted into every example `.s` now lives in a
single procedure.

### `fn` — pure, value-returning

```rust
fn iowr_hdr(op: byte, addr: int) -> bytes {
    [op, hi(addr), lo(addr)]
}
```

A call is replaced by its result:

```rust
send iowr_hdr(0x44, 0x0064) + crc8
```

is exactly `send [0x44, 0x00, 0x64] + crc8`. Use `fn` to build headers,
compute layouts, and compose with `crc8()` — all at compile time.

### Named arguments and defaults

Both callables take positional **and named** arguments, and parameters may have
defaults. Named arguments make a call self-documenting; defaults keep the common
case short.

```rust
proc command(pkt: bytes, ndata: int, err: byte = Verdict.Crc) {
    // …
}

command(pkt = [espi.PUT_OOB, 0x21, 0x00, 0x04], ndata = 0)   // err defaults to Verdict.Crc
command([espi.PUT_OOB, 0x21], 0, Verdict.Ok)                 // positional also works
```

**Pitfall — a `proc` is not free per call.** Because it inlines, ten calls to a
five-instruction `proc` emit fifty instructions. That is usually what you want
(no linkage exists), but keep the program-size cap (1024 words) in mind for large
unrolled loops.

---

## 8. Structured control flow

`tamal-lang` gives you `if`/`else`, `while`, `do`/`while`, and `repeat N`. It
gives you **no user-visible labels and no branch mnemonics.** The compiler owns
all branching.

### Why no labels or branches?

Two reasons, both about safety:

1. **Inlining would collide labels.** A `proc` inlined at three call sites would
   duplicate any label it defined, producing three `poll:` labels. The compiler
   sidesteps this by generating fresh (gensym'd) labels for every expansion —
   which it can only guarantee if *you* never write a label.
2. **Structured flow is the familiar, safer surface.** `if`/`while` cannot
   produce an unbalanced or unreachable branch the way hand-written `beq`/`j`
   can.

Raw *instruction* statements (`put_byte`, `tar`, `get_byte`, `mark`, `rdsr`, …)
stay 1:1 with their mnemonics — the wire stays visible. Only the *branch*
mnemonics (`beq`, `bne`, `j`, `beqz`, …) are withheld, and `--emit-asm` shows you
exactly the branches the compiler generated.

### `repeat N` — compile-time unroll

`repeat N { … }` repeats a block `N` times at compile time. There is no loop
counter, no branch — it literally emits the body `N` times.

```rust
repeat 3 { recv _ }
```

lowers to:

```asm
        get_byte x5
        get_byte x5
        get_byte x5
```

Use it for a fixed, known count (payload bytes, status bytes). `N` must be a
compile-time constant.

### `while`, `do`/`while`, `if`/`else` — real branches

These lower to the engine's branch instructions (`beq`, `bne`, `bltu`, `bgeu`)
plus unconditional jumps (`j`), with gensym'd labels. A `do { … } while r == K`
tests at the bottom:

```rust
do {
    crc_reset
    recv status
} while status == 0x0F      // keep going while the byte is WAIT_STATE
```

lowers to something like:

```asm
.Lloop0:
        crc_reset
        get_byte x5
        li x6, 0x0F
        beq x5, x6, .Lloop0     # while status == 0x0F
```

That pattern — reset residue, read a byte, loop while it is WAIT_STATE — is
exactly what `wait_state` ([§10](#wait_state)) packages for you. Conditions
compare a `reg` (or a compile-time value) against another value; the compiler
picks the matching branch mnemonic and reverses the sense as needed.

> The exact lowering of the WAIT_STATE poll — the hand-rolled `beq` loop shown
> here versus the ISA's native bounded `wait_on rd, cond, timeout` — is spec open
> question §8.1. The examples hand-roll the loop; a bounded timeout may be safer
> on the rig. This guide shows the hand-rolled form because that is what the
> committed `.s` files use.

---

## 9. The named-variable register model

You name your variables; the compiler assigns physical registers. You never
write `x5`.

### The rules

- The engine has **16 physical registers, `x0`..`x15`.** `x0` is hardwired to
  zero and is never allocated, leaving **15 usable** registers.
- Each `let`/`reg`/`recv` variable is allocated to a register for the duration of
  its lexical scope, then released.
- **Each `proc` expansion gets fresh registers (hygiene).** A procedure you call
  can never clobber a live value in the caller, because it is handed a disjoint
  set of registers.
- **More than 15 simultaneously-live variables is a compile error** that names
  the offending scope. There is no spill target in the ISA, so the compiler
  refuses rather than inventing memory that does not exist.
- The allocator never emits `x16`..`x31` (they do not exist in v1), never aliases
  a live variable, and never touches `x0`.

### Why hygiene matters

In the hand-written `.s`, three scratch registers (`t0`, `t1`, `t2`) are juggled
by hand across the whole program. Call a macro that also uses `t0` and you have
silently clobbered your value. The tamal-lang allocator makes that impossible:

```rust
proc poll_status() {
    reg s
    do { crc_reset; recv s } while s == 0x0F
}

test t {
    recv keep            // 'keep' is live across the call below
    poll_status()        // gets its OWN registers; cannot touch 'keep'
    mark 1, keep         // 'keep' is intact
}
```

Because `poll_status` is inlined with fresh registers, `keep` survives untouched.
The register-clobber hazard is solved by construction, not by discipline.

### Pitfall — exhaustion is a hard error

If a scope genuinely needs 16 live values at once, the compiler stops with a
diagnostic pointing at the scope. The fix is to shorten a variable's lifetime
(let it go out of scope sooner) or discard values you do not need (`recv _`), not
to reach for a spill that cannot exist.

---

## 10. Domain sugar

These are the eSPI-shaped conveniences. Each folds a copy-paste block into one
keyword, and each lowers to plain mnemonics you can read in `--emit-asm`.

### `config`

Sets the engine configuration. Lowers 1:1 to `set_config` with the same
keywords.

```rust
config controller, x1, sck20, alert_pin
```

→ `set_config controller, x1, sck20, alert_pin`.

The four fields are role (`controller` | `target`), IO width (`x1` | `x2` |
`x4`), clock (`sck20` | `sck33` | `sck50` | `sck66`), and alert source
(`alert_pin` | `alert_io1`). **v1 of the rig accepts only `controller, x1,
sck20, {alert_pin|alert_io1}`**; the other combinations are reserved and the
backend rejects them today. (Whether the front-end should mirror that
restriction or let the assembler reject unsupported combos is spec open question
§8.5.)

### `frame { … }`

Asserts CS# on entry and **guarantees CS# deasserts on every exit path** — normal
fall-through *and* a failing `expect`. This encodes the load-bearing eSPI
invariant "CS# deasserts before the verdict, verdict-independent," which is easy
to botch by hand.

```rust
frame {
    send [0x44, 0x00, 0x64] + crc8
    tar 2
    // …
}
```

The `cs_assert` and the guaranteed `cs_deassert` bracket the body:

```asm
        cs_assert
        # … body …
        cs_deassert        # emitted on every exit, including failure paths
```

You cannot leak an asserted CS# out of a `frame`. Think of it as RAII for the
chip-select line.

### `send`

Emits bytes. Three forms:

```rust
send [0x44, 0x00, 0x64]        // one put_byte per element
send pkt + crc8                // the bytes, then their appended CRC (§5)
send pkt ++ [crc8(pkt) ^ 0xFF] // the bytes, then a hand-controlled trailing byte (§6)
```

`send [a, b, c]` lowers to `put_byte a / put_byte b / put_byte c`. Elements may
be any compile-time byte expression; a `reg` element lowers to `put_byte <reg>`.

### `recv`

Reads the **post-response** bytes off the bus into named registers — the payload
and status bytes that follow the response code. (`wait_state` already consumed
the response code, and `expect crc` consumes the trailing CRC, so `recv` never
names those; see [read accounting](#a-note-on-read-accounting).) Use `_` to
**discard** a byte you do not need — a discard reuses one scratch register instead
of allocating a fresh one.

```rust
recv data, status0, status1     // three reads into three named registers
recv _                          // read one byte, discard it
repeat 4 { recv _ }             // read four bytes, discard all (see §8)
```

`recv a, b, c` lowers to a `get_byte` per target. Each `get_byte` also folds the
byte into the RX CRC-8 residue, which is what makes `expect crc` cheap.

The spec's §5 lowering table also lists a counted form (`recv N`); the concrete
library code uses `repeat N { recv _ }`, shown above, which this guide treats as
the canonical way to read a fixed count.

### `wait_state`

The reactive WAIT_STATE poll, folded into one word. eSPI targets stall by
returning the WAIT_STATE code (`0x0F`) repeatedly until they are ready; the
controller must keep reading until it sees something else. `wait_state` is that
loop:

```asm
.Lwait0:
        crc_reset               # drop the WAIT_STATE byte from the residue
        get_byte x5             # read a response byte
        li x6, 0x0F             # RSP_WAIT_STATE
        beq x5, x6, .Lwait0     # still WAIT_STATE → keep polling
```

Because `crc_reset` is *inside* the loop, only the final (non-WAIT_STATE)
response byte and everything after it contribute to the residue — which is
exactly what the CRC check needs. `wait_state` **consumes the response code**; the
bytes you `recv` afterward are the payload and status. (See
[read accounting](#a-note-on-read-accounting).)

#### Optionally binding the response byte: `wait_state name`

By default `wait_state` discards the terminal (non-WAIT_STATE) response byte — the
common case, since most tests only care whether the packet's CRC checked out. When
a conformance test needs to **assert the completion type** (that the target
answered `ACCEPT` and not some other code), give `wait_state` a name and it binds
that terminal byte to a `reg`:

```rust
wait_state resp          // bind the response code instead of discarding it
mark 1, resp             // now you can observe or check it
```

The default `wait_state` and `wait_state resp` emit the **same poll loop and the
same byte stream** — the only difference is whether the terminal byte lands in a
named register or a scratch one. Binding it costs one register, not one bus cycle,
so the wire behavior is identical either way.

### `expect crc else <byte>`

Reads the trailing CRC byte, checks that the RX CRC-8 **residue is `0`** (the
signature of a well-formed packet), and — if it is not — writes the given verdict
byte and halts. Crucially, the `frame`'s `cs_deassert` runs first, so the verdict
is issued with CS# already high.

```rust
frame {
    // … send, tar, wait_state, recv …
    expect crc else 0x11
}
pass
```

lowers to:

```asm
        get_byte x5            # trailing CRC byte → residue
        rdsr x7, crc           # read the RX CRC-8 residue
        cs_deassert            # frame exit: CS# high before the verdict (D9)
        bnez x7, .Lfail0       # residue != 0 → take the failure path
        halt 0x00              # pass  (fall-through)
.Lfail0:
        halt 0x11              # expect … else 0x11
```

Residue `0` means the packet's own CRC checked out; any non-zero residue means
corruption, and you get verdict `0x11`. The verdict byte is the only thing that
varies between channels — the skeleton is identical everywhere, which is why it
is sugar.

### `pass` and `fail`

The verdict writers. Both halt.

```rust
pass          // → halt 0x00
fail 0x11     // → halt 0x11
```

`pass` is the success verdict (`0x00`); `fail X` halts with your chosen non-zero
code. A `test` must reach one of them (directly, or via an `expect`).

### `mark`

A raw engine instruction (lowers 1:1), listed here because you will reach for it
constantly for observability. It streams `{tag, register-payload}` into the trace
ring — the tamal `printf`.

```rust
let v = 0x123456
mark 3, v      // → mark 3, x<n> ; tag must be 0..2047
```

The tag is a compile-time number in `0..2047`; the payload is a register's full
32-bit value. See [`examples/mark_trace.s`](../../examples/mark_trace.s).

---

## 11. `test`

`test NAME { … }` declares the program's single entry point. A `test` *is* the
entry and the text section.

```rust
test io_read {
    // …
    pass
}
```

lowers to:

```asm
        .globl _start
_start:
        # … body …
        halt 0x00      # from the final pass
```

Every path through a `test` must reach a `halt` — via `pass`, `fail`, or a
failing `expect`. The compiler checks this; a test that can fall off the end is
an error.

---

## 12. Imports and modules

One file is one module. Items are **private unless marked `pub`.** You bring
another module in with `import`, and you refer to its items with a qualified
name — imported names never silently leak into your namespace.

### The two import forms

```rust
import espi                    // the bundled stdlib module, namespaced as `espi`
import "lib/foo.tam" as foo    // a path import, namespaced as `foo`
```

References are always qualified:

```rust
espi.PUT_OOB        // a const from the espi module
espi.command(...)   // a proc from the espi module
foo.helper(...)     // a proc from your own library
```

### Declaring a public API

```rust
// lib/foo.tam
pub const MAGIC = 0xAB           // visible to importers
const INTERNAL = 0x00            // private to this module

pub proc helper(x: byte) {       // visible
    put_byte x
}
```

Only `pub` items are reachable through an import; everything else is module-local.

### Resolution is deterministic

Name resolution order is **local → module → qualified import**. Modules are
loaded from a search path (the importer's directory, `-I`/`-L` paths, then the
bundled stdlib). And it is deterministic by design, because the rig must be
reproducible:

- Paths are **canonicalized and deduped by identity**, so the same file imported
  two ways is one module.
- **Import cycles are a hard error.**
- A **duplicate definition with a different body is a hard error** (two identical
  bodies may dedupe; two different bodies that claim the same name may not).

These rules defend against the classic source-form-library hazards: flat-dump
name collisions, include-order-dependent output, and diamond-import skew. You get
the same bytecode no matter how the import graph is walked.

The exact stdlib surface and search-path precedence (`$TAMAL_LIB` vs `-L` vs
vendored) is spec open question §8.4; see [stdlib-espi.md](stdlib-espi.md) for
the v1 `espi` surface as designed.

---

## A note on read accounting

Three constructs share the reading of a response phase, and it is worth seeing
exactly which byte each one takes so your `recv` counts are always right.

The response phase of a channel read has this byte layout on the wire:

```
[ WAIT_STATE… ] [ response code ] [ payload… ] [ status0 ] [ status1 ] [ CRC ]
```

The division of labor:

1. `wait_state` reads response bytes in a loop and **consumes the response
   code** (the first non-WAIT_STATE byte). That read is intrinsic — you cannot
   detect WAIT_STATE without reading the byte. By default the byte is discarded;
   `wait_state name` binds it (see [above](#wait_state)).
2. `recv …` reads the **payload and status** bytes, one `get_byte` per target.
3. `expect crc` reads the **trailing CRC byte**, then checks the residue.

So the number of `recv` targets is *(payload bytes) + 2 status bytes* — you never
count the response code (which `wait_state` took) or the CRC (which `expect`
takes). For the channel examples:

| Example | payload | `recv` targets | trailing CRC |
|---|---|---|---|
| `peripheral_io_read` | 1 (the I/O byte) | `data, status0, status1` (3) | `expect crc` |
| `oob_smbus_msg` (write completion) | 0 | `status0, status1` (2) | `expect crc` |

This is the rule fixed by design decision **D12**: `wait_state` owns the response
code, `recv` owns the payload and status, and `expect crc` owns the trailing CRC.
The `command` proc in the `espi` library encodes it directly — it reads `ndata`
payload bytes, then two status bytes, then checks the residue — so calling it with
the right `ndata` (`1` for the peripheral read, `0` for the write completions) is
all you ever need to get the counts right.

---

## Where to go next

- The eSPI library you keep importing → [stdlib-espi.md](stdlib-espi.md).
- Copy-pasteable recipes → [cookbook.md](cookbook.md).
- Fast lookup (keywords, operators, the full lowering table, CLI) →
  [reference.md](reference.md).
