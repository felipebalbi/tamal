# Getting started with tamal-lang

> **Status:** designed, not yet implemented. You cannot run `tamalc` today; this
> tutorial shows the designed language and the assembly it is specified to
> produce. Every assembly block below is real tamal-asm that the existing
> `tamal-asm` backend already accepts, so you can sanity-check the *lowering* by
> assembling it by hand.

This is a five-minute tour. Three steps, one new idea each, and every step maps
to an example program that already ships in `examples/`. By the end you will
have written a real eSPI Peripheral-channel I/O read.

The trick you will use at every step is `tamalc --emit-asm`: it prints the exact
tamal assembly your `.tam` compiles to. That is your **trust bridge** — nothing
is hidden, and you can read the wire directly.

---

## Step 0 — the mental model

Keep this sentence in your head:

> A `.tam` file is a list of **tests**; a test is a sequence of **frames**; a
> frame is the bytes you **send** and **recv** between CS-assert and
> CS-deassert; **pass**/**fail** write the verdict byte.

Everything below is that sentence, filled in.

---

## Step 1 — the smallest test: `pass`

The smallest useful tamal program halts immediately with the "ok" verdict. It
exercises nothing but the load → trigger → halt → trace-drain round-trip — the
first thing you run on a freshly flashed board.

```rust
// smoke.tam
test smoke {
    pass
}
```

- `test smoke { … }` declares the single entry point. A `test` *is* the program
  entry and the text section; you never write `.globl _start` or `_start:`
  yourself.
- `pass` writes verdict byte `0x00` and halts.

Compile it and look at the assembly:

```console
$ tamalc compile smoke.tam --emit asm
        .globl _start
_start:
        halt 0x00          # pass
```

That is **byte-for-byte identical** to the hand-written
[`examples/smoke_halt.s`](../../examples/smoke_halt.s), whose whole body is
`halt VERDICT_OK` (and `VERDICT_OK = 0x00`). You just wrote it without the
boilerplate.

**What you learned:** a test is an entry point; `pass` is `halt 0x00`.

---

## Step 2 — say something back: `mark`

With no logic analyzer attached, `mark` is how the engine talks back. Each
`mark` streams a tagged 32-bit register value into the trace ring; the host
loader prints one line per record. It is the tamal equivalent of a `printf`
probe.

```rust
// hello.tam
test hello {
    let a = 0xDEAD        // a compile-time value, bound to a name
    mark 1, a             // stream {tag = 1, payload = a} into the trace
    pass
}
```

Two new ideas:

- `let a = 0xDEAD` binds a name to a value. Because `0xDEAD` is a compile-time
  constant, the compiler loads it into a register for you.
- `mark 1, a` is a raw engine instruction — it lowers 1:1, unchanged. The first
  operand is a numeric **tag** (`0..2047`); the second is the register whose
  32-bit value is captured.

```console
$ tamalc compile hello.tam --emit asm
        .globl _start
_start:
        li x1, 0xDEAD      # let a = 0xDEAD
        mark 1, x1         # mark 1, a
        halt 0x00          # pass
```

The compiler assigned `a` to register `x1` (the first free register). **You never
choose the register number** — the allocator does, and `--emit-asm` shows you
which it picked. `li x1, 0xDEAD` is one instruction because `0xDEAD` fits the
engine's signed-21-bit immediate; a larger value would tile into `lui`+`addi`,
and the assembler handles that, not you.

This is the same machinery as
[`examples/mark_trace.s`](../../examples/mark_trace.s), which marks three values
(`0xDEAD`, `0xBEEF`, `0x123456`) to prove computed register state round-trips
over UART, in order, ahead of the terminating `HALT`. Try adding two more `let`
+ `mark` pairs and re-running `--emit-asm`.

**What you learned:** `let` names a value; `mark tag, reg` streams it back;
registers are allocated for you.

---

## Step 3 — a real transaction: a Peripheral I/O read

Now the payoff. We read one byte from I/O port `0x64` (the classic 8042
keyboard-controller status port) using the eSPI `PUT_IORD_SHORT` command, then
verify the completion's CRC. This is the full command/response/turnaround dance,
and it is exactly
[`examples/peripheral_io_read.s`](../../examples/peripheral_io_read.s) — but
without the 50 lines of framing, polling, and hand-computed CRC.

```rust
// peripheral_io_read.tam — eSPI Peripheral channel: short I/O read
import espi

test io_read {
    config controller, x1, sck20, alert_pin

    frame {                                         // CS# low … CS# high (RAII)
        send [espi.PUT_IORD1, 0x00, 0x64] + crc8    // command phase — the only bytes that vary
        tar 2                                       // legal turnaround
        wait_state                                  // poll past WAIT_STATE; consumes the response-code byte
        recv data, status0, status1                 // response phase: data byte + 2 status bytes
        expect crc else 0x11                        // consumes trailing CRC byte; residue == 0, else verdict 0x11
    }
    pass                                            // halt 0x00
}
```

Seven new ideas, each earning its keep:

| Line | What it does |
|---|---|
| `import espi` | Brings in the eSPI vocabulary (`espi.PUT_IORD1 = 0x44`, verdict codes, helpers). eSPI opcodes live in a library, not the language. |
| `config controller, x1, sck20, alert_pin` | Sets the engine role/IO/clock/alert. Lowers 1:1 to `set_config`. |
| `frame { … }` | Asserts CS# on entry and **guarantees CS# deasserts on every exit** — including a failing `expect`. You cannot forget it. |
| `send […] + crc8` | Emits the command bytes, then appends their CRC-8, **recomputed at compile time** over exactly those bytes. |
| `wait_state` | The reactive WAIT_STATE poll idiom, folded into one word. It **consumes the response-code byte** (that is how it detects WAIT_STATE), so `recv` reads only what follows. |
| `recv data, status0, status1` | Reads the three post-response bytes into named registers. |
| `expect crc else 0x11` | Consumes the trailing CRC byte, checks the RX residue is `0`, and picks the verdict byte if it is not. |

Now the trust bridge — what it compiles to:

```console
$ tamalc compile peripheral_io_read.tam --emit asm
        .globl _start
_start:
        set_config controller, x1, sck20, alert_pin   # config …
        cs_assert                                      # frame {
        put_byte 0x44                                  #   send [PUT_IORD1,
        put_byte 0x00                                  #         0x00,
        put_byte 0x64                                  #         0x64]
        put_byte 0x16                                  #   + crc8  = crc8([0x44,0x00,0x64])
        tar 2                                          #   tar 2
.Lwait0:                                               #   wait_state {
        crc_reset                                      #     drop WAIT_STATE bytes from the residue
        get_byte x5                                    #     read the response code
        li x6, 0x0F                                    #     RSP_WAIT_STATE
        beq x5, x6, .Lwait0                            #   } keep polling while WAIT_STATE
        get_byte x5                                    #   recv → data
        get_byte x5                                    #   recv → status0
        get_byte x5                                    #   recv → status1
        get_byte x5                                    #   expect crc: trailing CRC byte → residue
        rdsr x7, crc                                   #   read RX CRC-8 residue
        cs_deassert                                    # } frame exit: CS# high, verdict-independent
        bnez x7, .Lfail0                               #   expect crc else 0x11
        halt 0x00                                      # pass
.Lfail0:
        halt 0x11                                      # (expect … else 0x11)
```

Look at `put_byte 0x16`. You never typed `0x16`. The compiler folded
`crc8([0x44, 0x00, 0x64])` — using *the exact same CRC function that runs on the
wire and in the HDL* — and emitted the byte. Edit any of the three command bytes
and the `0x16` updates on the next build. The single most error-prone line in
the hand-written `.s` is now correct-by-construction.

This assembly is the transaction in
[`examples/peripheral_io_read.s`](../../examples/peripheral_io_read.s),
instruction for instruction. (The exact scratch-register numbers match too when
the allocator reuses one register for the discarded reads, the way the `.s`
reuses `t0` — see [cookbook → verify
byte-for-byte](cookbook.md#recipe-8-verify-byte-for-byte-equivalence).)

> **How the reads line up.** `wait_state` already *reads* the response code —
> that is how it detects WAIT_STATE — so it **consumes the response-code byte**
> and, by default, discards it. That leaves exactly three bytes for `recv`
> (`data`, `status0`, `status1`), and `expect crc` consumes the trailing CRC
> byte. So `recv` names only the post-response bytes; you never count the
> response code as a `recv` target. If a test needs to *assert* the completion
> type, `wait_state name` binds that response byte instead of discarding it (see
> the [language guide](language-guide.md#wait_state)).

**What you learned:** `import`, `config`, `frame`, `send … + crc8`,
`wait_state`, `recv`, `expect crc`, `pass` — the whole spine of a channel test.

---

## Where to go next

- You want the eSPI opcodes and helpers → [stdlib-espi.md](stdlib-espi.md).
- You want every construct explained → [language-guide.md](language-guide.md).
- You want working recipes to copy → [cookbook.md](cookbook.md).
- You want a fast keyword/operator/lowering lookup → [reference.md](reference.md).

Two habits worth forming now:

1. **Run `--emit-asm` often.** It is the fastest way to build trust and to learn
   what each construct costs.
2. **Let `+ crc8` compute your CRCs.** Never hand-type a good CRC byte again. When
   you *want* a bad one — for a negative test — write it loudly:
   `pkt ++ [crc8(pkt) ^ 0xFF]`.
