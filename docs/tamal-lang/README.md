# tamal-lang

A small, teachable high-level language for writing **eSPI compliance and
conformance tests** on the tamal rig. Source files end in `.tam`; the compiler
is `tamalc`.

> **The one-sentence mental model**
>
> A `.tam` file is a list of **tests**; a test is a sequence of **frames**; a
> frame is the bytes you **send** and **recv** between CS-assert and
> CS-deassert; **pass**/**fail** write the verdict byte.

Everything else in this documentation is an elaboration of that sentence.

---

## Status

> **Designed & approved; compiler not yet implemented.**
>
> `tamal-lang` has a committed design but no working `tamalc` binary yet. These
> documents describe the **designed** syntax and its lowering to tamal assembly.
> The authoritative design is
> [`docs/superpowers/specs/2026-07-20-tamal-lang-design.md`](../superpowers/specs/2026-07-20-tamal-lang-design.md).
>
> Where the design leaves a detail open, this documentation says so out loud and
> points at the spec's open questions (§8) rather than inventing an answer. Any
> code sample marked *"lowers to"* shows the **intended** assembly; you cannot
> run these through `tamalc` today, but every assembly snippet is real tamal-asm
> that the existing `tamal-asm` backend already accepts.

---

## Why this language exists

The tamal engine on the FPGA is a **dumb SPI shifter with zero eSPI knowledge**.
It shifts host-built bytes; it knows nothing about channels, cycle types, or
opcodes. The guiding principle of `tamal-lang` is: **so does the language.**

The language core gives you framing, CRC, verdicts, structured control flow, and
raw wire bytes. *eSPI itself* — command opcodes, cycle types, verdict codes —
lives in an importable, **source-form** library (`espi`) that you can read,
fork, and extend. Three consequences follow:

- **The wire stays visible.** Hardware engineers can read
  `tamalc --emit-asm` line-for-line against a logic-analyzer capture. Nothing is
  hidden; only boilerplate is folded.
- **The core stays tiny.** Software engineers learn the whole language from one
  page.
- **Everything is compile-time deterministic.** The same source compiles to
  byte-for-byte identical bytecode, every time, on every machine. The rig stays
  reproducible.

Concretely, `tamal-lang` kills the copy-paste boilerplate that is identical
across every `examples/*.s` — CS framing, the WAIT_STATE poll, the CRC-residue
verdict, the halt codes — and it makes the hand-computed TX CRC byte
**correct-by-construction** through a compile-time `crc8()` that is *the same
code* as the on-wire CRC and the HDL CRC.

Here is the whole story in one before/after. This hand-written assembly:

```asm
    put_byte 0x44          # CMD: PUT_IORD_SHORT
    put_byte 0x00          # addr[15:8]
    put_byte 0x64          # addr[7:0]
    put_byte 0x16          # TX CRC-8 over the 3 bytes above  <-- hand-computed, error-prone
    tar 2
```

becomes this, where the CRC can never drift out of sync with the bytes it
covers:

```rust
send [espi.PUT_IORD1, 0x00, 0x64] + crc8   // + crc8 recomputes on every byte edit
tar 2
```

---

## Table of contents

| Document | What it covers | Read it if you are… |
|---|---|---|
| [getting-started.md](getting-started.md) | A 5-minute tutorial: `pass`, `mark`, then a full peripheral I/O read, each mapped to an existing smoke test. | new to tamal-lang and want to write a first test now. |
| [language-guide.md](language-guide.md) | The complete teaching reference: every construct, with a worked example, its assembly lowering, and its pitfalls. | learning the language properly, or reviewing someone's `.tam`. |
| [stdlib-espi.md](stdlib-espi.md) | The bundled `espi` library — constants, enums, the `controller`/`command` procs, and how to import and fork it. | writing real channel tests and want the eSPI vocabulary. |
| [reference.md](reference.md) | Terse lookup: keywords, builtins, operators, the full sugar→asm lowering table, a grammar sketch, and `tamalc` CLI usage. | you know the language and need to check one detail fast. |
| [cookbook.md](cookbook.md) | Copy-pasteable recipes: the four channel tests, deliberate-bad-CRC and illegal-TAR negatives, writing your own library, and verifying byte-for-byte equivalence. | you have a job to do and want a working starting point. |
| [for-hardware-engineers.md](for-hardware-engineers.md) | A bridge from Verilog/SV/VHDL and assembly: the wire never hides. | you think in clocks, pins, and cycles. |
| [for-software-engineers.md](for-software-engineers.md) | A bridge from Python/C/Rust/JS: what is familiar, what is deliberately missing, and why. | you think in functions, types, and modules. |

---

## The shape of a test

```rust
// hello.tam
import espi                       // pull in the eSPI vocabulary

test io_read {                    // one test = one entry point
    config controller, x1, sck20, alert_pin

    frame {                       // CS# low … CS# high, guaranteed on every exit
        send [espi.PUT_IORD1, 0x00, 0x64] + crc8   // the command bytes + auto CRC
        tar 2                                       // turnaround (always an explicit width)
        wait_state                                  // poll past WAIT_STATE (consumes the response byte)
        recv data, status0, status1                 // read the post-response bytes
        expect crc else 0x11                        // consumes trailing CRC; residue must be 0, else verdict 0x11
    }
    pass                          // halt 0x00
}
```

Read that top to bottom and you have read the language. The rest is detail.

---

## Two commitments this language makes

1. **The compile-time CRC cannot drift.** `crc8()` is literally
   `tamal_abi::crc8` — the exact function used on the wire and mirrored in the
   HDL `Tamal.Crc`. When you edit a payload byte, `+ crc8` recomputes; there is
   no second hand-maintained transcription to fall out of date.
2. **Danger is always loud.** A compliance rig must send bad CRCs and illegal
   turnarounds *on purpose*. `tamal-lang` never silently legalizes an intended
   violation: a deliberately wrong CRC is written `pkt ++ [crc8(pkt) ^ 0xFF]`
   (greppable, obviously intentional), and an illegal turnaround is always an
   explicit width like `tar 3` — never hidden inside a helper.

Both are covered in depth in the [language guide](language-guide.md) and the
[cookbook](cookbook.md).
