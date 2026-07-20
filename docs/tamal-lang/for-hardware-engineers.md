# tamal-lang for hardware engineers

> **Status:** designed, not yet implemented. This is a bridge from Verilog/SV/
> VHDL and assembly into `tamal-lang`. If a construct here sounds like it might
> hide the wire, read to the end of its paragraph — it does not.

You already think in clocks, pins, turnaround, and cycles. `tamal-lang` is built
for exactly that mindset. The one thing to internalize: **the language has no
more eSPI knowledge than the engine does.** The tamal engine is a dumb SPI
shifter that shifts host-built bytes; the language builds those bytes and folds
the boilerplate around them. It never invents protocol behavior, and it never
hides a byte.

---

## The wire never hides

Your instinct is right to distrust an abstraction that sits between you and the
bus. So here is the deal: `tamalc compile … --emit asm` prints the exact tamal
assembly — every `put_byte`, `tar`, `get_byte`, `cs_assert` — that your `.tam`
becomes. It is your netlist-after-synthesis: the thing you actually read against
a logic-analyzer capture.

```rust
send [espi.PUT_IORD1, 0x00, 0x64] + crc8
tar 2
```

```asm
        put_byte 0x44          # PUT_IORD_SHORT
        put_byte 0x00          # addr[15:8]
        put_byte 0x64          # addr[7:0]
        put_byte 0x16          # + crc8  = crc8([0x44,0x00,0x64])
        tar 2                  # turnaround, explicit width
```

Nothing was added, nothing hidden. The only line you did not type is the CRC —
and that is the point.

---

## What each construct is, in your terms

| tamal-lang | Hardware analogy |
|---|---|
| `frame { … }` | a bus-transaction bracket: `cs_assert` on entry, `cs_deassert` guaranteed on every exit — you cannot leak an asserted CS# |
| `send [ … ]` | a run of `put_byte` — bytes shifted onto `IO[0]` MSB-first |
| `recv a, b` | a run of `get_byte` — bytes sampled off the bus into registers |
| `tar 2` | the turnaround/tri-state cycle count — **always an explicit width**, never inferred |
| `wait_state` | the reactive WAIT_STATE (`0x0F`) poll loop; the target stalls, you keep clocking until it answers |
| `expect crc else X` | read the trailing CRC, check the RX residue is `0`, pick a verdict byte otherwise |
| `mark tag, r` | a trace probe — streams a tagged 32-bit register value to the host, your `$display`/ILA marker |
| `config …` | `set_config`: role, IO width, SCK, alert source |

Turnaround gets special mention because it is where the dangerous timing lives.
`tar` is never folded into a helper and never normalized — `tar 2` is legal,
`tar 3` is an intentional violation, and both are driven exactly as written. The
language will not "help" you by fixing a turnaround width.

---

## The CRC is your HDL's CRC

The reliability headline is aimed straight at you. The compile-time `crc8()` is
**the same code** as the on-wire CRC and the HDL `Tamal.Crc` — CRC-8, poly
`0x07`, init `0x00`, MSB-first, no reflection, no final XOR. Not "a compatible
reimplementation." The same function.

So when `send pkt + crc8` folds `0x16` into the stream, that byte cannot disagree
with what the target's CRC block computes, because both are the one
implementation. The residue law holds: `crc8(msg ++ [crc8(msg)]) == 0`, which is
why `expect crc` just checks for a zero residue. Edit a payload byte and the TX
CRC recomputes — no stale hand-computed byte to fall out of sync.

When you *want* a bad CRC (to confirm the DUT rejects it), you write it loudly:

```rust
send pkt ++ [crc8(pkt) ^ 0xFF]     // greppable intent; 0x16 ^ 0xFF = 0xE9
```

---

## The datapath you are targeting

The engine is deliberately small; the language respects its limits rather than
papering over them:

- **16 registers, `x0`..`x15`** (`x0` hardwired zero). You name variables; the
  compiler allocates registers and shows them in `--emit-asm`. More than 15 live
  at once is a compile error — there is **no spill**, because there is no data
  memory to spill to.
- **No `call`/`ret`, no stack.** A `proc` is *inlined* (like an always-expanded
  macro), not called. Each inlining gets fresh registers, so a helper can never
  clobber your live values.
- **Program ≤ 1024 words, branch reach ±1024 words.** The same limits the
  assembler already enforces; the compiler reports them against your `.tam`
  source.
- **Structured control flow only.** You write `if`/`while`/`do-while`/`repeat`;
  the compiler emits `beq`/`bne`/`bltu`/`bgeu`/`j` with generated labels. Raw
  non-branch mnemonics (`put_byte`, `tar`, `rdsr`, …) are still available — only
  the branch mnemonics are compiler-owned, to keep inlined labels from colliding.

`repeat N { … }` is a compile-time unroll — no loop counter, no run-time branch —
so it behaves like a `generate` loop, not a `for` with a live index.

---

## Determinism, because it is a test rig

Same source in, byte-identical bytecode out — every machine, every run. The
compiler reads no clock, no environment, no RNG; nothing that reaches the bytes
is order-dependent. That is what lets a conformance failure be *replayed* exactly.
(The deferred error-injection feature keeps this property: `ratio = 0` is
structurally zero, and a given `(seed, ratio)` replays byte-for-byte — no
run-time randomness ever.)

---

## Where to go next

- See it end to end → [getting-started.md](getting-started.md).
- The full construct list with lowerings → [language-guide.md](language-guide.md).
- Prove the abstraction changed no bytes →
  [cookbook.md → verify byte-for-byte](cookbook.md#recipe-8-verify-byte-for-byte-equivalence).
