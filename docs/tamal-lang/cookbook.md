# tamal-lang cookbook

> **Status:** designed, not yet implemented. These recipes show designed `.tam`
> and the assembly they are specified to produce. The assembly is real
> tamal-asm; the `.tam` awaits `tamalc`. CRC bytes shown (`0x16`, `0xB1`,
> `0x89`, `0xE8`) are the actual `tamal_abi::crc8` values and match the
> hand-written `examples/*.s`.

Copy a recipe, change the bytes, rebuild. Each recipe names the example `.s` it
corresponds to so you can diff against the committed ground truth
([Recipe 8](#recipe-8-verify-byte-for-byte-equivalence)).

**Recipes**

1. [Peripheral I/O read](#recipe-1-peripheral-io-read)
2. [OOB tunneled SMBus message](#recipe-2-oob-tunneled-smbus-message)
3. [Virtual Wire: deassert PLTRST#](#recipe-3-virtual-wire-deassert-pltrst)
4. [Flash read completion](#recipe-4-flash-read-completion)
5. [Deliberate bad CRC (negative test)](#recipe-5-deliberate-bad-crc)
6. [Deliberate illegal turnaround](#recipe-6-deliberate-illegal-turnaround)
7. [Write your own library](#recipe-7-write-your-own-library)
8. [Verify byte-for-byte equivalence](#recipe-8-verify-byte-for-byte-equivalence)

---

## Recipe 1: Peripheral I/O read

Corresponds to [`examples/peripheral_io_read.s`](../../examples/peripheral_io_read.s).
Read one byte from I/O port `0x64` and verify the completion CRC. This is spec
§4.2 verbatim.

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

The appended CRC folds to `0x16` (`crc8([0x44, 0x00, 0x64])`). See the
[getting-started walkthrough](getting-started.md#step-3--a-real-transaction-a-peripheral-io-read)
for the full `--emit-asm`, and the [read-accounting
note](language-guide.md#a-note-on-read-accounting) for why `recv` names exactly
the three post-response bytes (`wait_state` took the response code; `expect crc`
takes the trailing CRC).

---

## Recipe 2: OOB tunneled SMBus message

Corresponds to [`examples/oob_smbus_msg.s`](../../examples/oob_smbus_msg.s).
Built from the shared `espi.command` proc. This is spec §4.3 verbatim.

```rust
// oob_smbus_msg.tam — OOB channel, built from the shared library
import espi

test oob_msg {
    espi.controller()
    espi.command(
        pkt   = [espi.PUT_OOB, 0x21, 0x00, 0x04,   // eSPI OOB header
                 0x10, 0x00, 0x01, 0xAB],          // tunneled SMBus {dest,cmd,count,data}
        ndata = 0,                                 // a write completion returns status only, no payload
    )
    pass
}
```

The CRC folds to `0xB1`. The full lowering and the `ndata` explanation are in
[stdlib-espi.md](stdlib-espi.md#a-complete-test-built-from-the-library): an OOB
*write* completion carries no payload, so `ndata = 0` (the response phase is the
poll, two status bytes, and the CRC).

---

## Recipe 3: Virtual Wire: deassert PLTRST#

Corresponds to
[`examples/virtual_wire_pltrst.s`](../../examples/virtual_wire_pltrst.s). Drive a
single Virtual Wire group that deasserts platform reset (`PLTRST#`).

```rust
// virtual_wire_pltrst.tam — eSPI Virtual Wire channel
import espi

test vw_pltrst {
    espi.controller()
    espi.command(
        pkt   = [espi.PUT_VWIRE, 0x00, 0x03, 0x22],  // count=0; VW index 0x03; data 0x22 = PLTRST# high
        ndata = 0,                                   // a VW write completion returns no payload
    )
    pass
}
```

The CRC folds to `0x89` (`crc8([0x04, 0x00, 0x03, 0x22])`), matching the
hand-written `0x89`. Lowers to:

```asm
        .globl _start
_start:
        set_config controller, x1, sck20, alert_pin   # espi.controller()
        cs_assert                                      # command → frame {
        put_byte 0x04                                  #   send [PUT_VWIRE,
        put_byte 0x00                                  #         0x00,
        put_byte 0x03                                  #         0x03,
        put_byte 0x22                                  #         0x22]
        put_byte 0x89                                  #   + crc8 = crc8(pkt)
        tar 2                                          #   tar 2
.Lwait0:                                               #   wait_state {
        crc_reset
        get_byte x5
        li x6, 0x0F
        beq x5, x6, .Lwait0                            #   }
        get_byte x5                                    #   recv status0
        get_byte x5                                    #   recv status1
        get_byte x5                                    #   expect crc: trailing CRC → residue
        rdsr x7, crc
        cs_deassert                                    # } frame exit
        bnez x7, .Lfail0                               #   expect crc else Verdict.Crc (0x11)
        halt 0x00                                      # pass
.Lfail0:
        halt 0x11
```

> `espi.PUT_VWIRE` (`0x04`) is defined in the full `espi` library; the excerpt in
> [stdlib-espi.md](stdlib-espi.md) shows only `PUT_IORD1` and `PUT_OOB`. If you
> are working against the excerpt, add `const PUT_VWIRE = 0x04` locally or use
> the literal `0x04`.

---

## Recipe 4: Flash read completion

Corresponds to [`examples/flash_completion.s`](../../examples/flash_completion.s).
Return a 4-byte flash-read completion carrying `{0xDE, 0xAD, 0xBE, 0xEF}`.

```rust
// flash_completion.tam — eSPI Flash Access channel: return a read completion
import espi

test flash_cmpl {
    espi.controller()
    espi.command(
        pkt   = [espi.PUT_FLASH_C, 0x09, 0x00, 0x04,  // PUT_FLASH_C; Successful Completion With Data; len=4
                 0xDE, 0xAD, 0xBE, 0xEF],             // the flash data being returned
        ndata = 0,                                    // the ACCEPT we read back carries only status
    )
    pass
}
```

The CRC folds to `0xE8` (`crc8([0x08, 0x09, 0x00, 0x04, 0xDE, 0xAD, 0xBE,
0xEF])`), matching the hand-written `0xE8`. The lowering is identical in shape to
Recipe 3, with the eight `put_byte` values above and `put_byte 0xE8` for the CRC.

> `espi.PUT_FLASH_C` (`0x08`) is in the full library; see the note in Recipe 3.

---

## Recipe 5: Deliberate bad CRC

A compliance rig must send corrupt packets on purpose to confirm the DUT rejects
them. Write the corruption **loudly** — never a bare wrong literal.

```rust
import espi

test io_read_badcrc {
    config controller, x1, sck20, alert_pin

    let pkt = [espi.PUT_IORD1, 0x00, 0x64]

    frame {
        send pkt ++ [crc8(pkt) ^ 0xFF]     // deliberately wrong: 0x16 ^ 0xFF = 0xE9
        tar 2
        wait_state
        recv data, status0, status1
        expect crc else 0x11
    }
    pass
}
```

The command phase lowers to:

```asm
        put_byte 0x44
        put_byte 0x00
        put_byte 0x64
        put_byte 0xE9          # crc8(pkt) ^ 0xFF — deliberately wrong (0x16 ^ 0xFF)
```

Why this is the right way:

- `++ [ … ]` (explicit concat) instead of `+ crc8` (correct-by-construction)
  signals "hand-controlled trailing byte."
- The `^ 0xFF` is **greppable** — you can find every intentional corruption in
  the tree.
- `--lint` leaves this alone but would flag an *accidental* stale CRC literal.

See [language guide §6](language-guide.md#6-deliberate-wrong).

---

## Recipe 6: Deliberate illegal turnaround

Turnaround width is always an explicit number. The legal width for these
transfers is `tar 2`; an intentional violation is simply a different width.

```rust
frame {
    send [espi.PUT_IORD1, 0x00, 0x64] + crc8
    tar 3                          // deliberate TAR violation (legal is tar 2)
    wait_state
    recv data, status0, status1
    expect crc else 0x11
}
```

`tar 3` lowers, unchanged, to `tar 3`. The `tar` field encodes `0..15`, so the
assembler accepts the width; it is the *eSPI protocol* that `tar 3` violates —
which is exactly what you are testing. No helper can silently normalize it back
to `2`, because turnaround is never hidden inside a helper.

---

## Recipe 7: Write your own library

Factor your rig's conventions into a module. Mark the public surface `pub`; keep
helpers private.

```rust
// lib/myrig.tam — project-specific helpers

pub const PUT_IORD1 = 0x44
const WAIT_STATE    = 0x0F          // private: an implementation detail

pub enum V: byte { Ok = 0x00, BadCrc = 0x11, Timeout = 0x22 }

// A read that names its verdict codes.
pub proc io_read(port: int) {
    frame {
        send [PUT_IORD1, hi(port), lo(port)] + crc8
        tar 2
        wait_state
        recv data, status0, status1
        expect crc else V.BadCrc
    }
}
```

Use it by path import, aliased to whatever name you like:

```rust
import "lib/myrig.tam" as rig

test port64 {
    config controller, x1, sck20, alert_pin
    rig.io_read(port = 0x0064)      // qualified call; named argument
    pass
}
```

Rules that keep this deterministic (see
[language guide §12](language-guide.md#12-imports-and-modules)):

- Only `pub` items are visible through the import; `WAIT_STATE` stays private.
- References are qualified (`rig.io_read`, `rig.V.BadCrc`) — no name leaks.
- The importer's directory, then `-I`/`-L`, then the vendored stdlib are
  searched; paths are canonicalized and deduped; cycles and
  duplicate-different-body definitions are hard errors.

---

## Recipe 8: Verify byte-for-byte equivalence

The success criterion for these tests is that the HLL compiles to the *same
bytecode* as the hand-written `.s`. You verify it yourself with two builds and a
compare.

```sh
# Build the HLL test to bytecode.
tamalc compile peripheral_io_read.tam --emit bin -o hll.bin

# Build the reference assembly to bytecode.
tamal-asm assemble examples/peripheral_io_read.s --emit bin -o ref.bin

# They must be identical.
cmp hll.bin ref.bin && echo "byte-identical"
```

To *understand* a difference (or confirm a match line by line), compare the
assembly instead of the bytes:

```sh
tamalc compile peripheral_io_read.tam --emit asm > hll.s
diff <(sed 's/#.*//' hll.s) <(sed 's/#.*//' examples/peripheral_io_read.s)
```

(Stripping `#` comments compares only the instructions; label *names* and
comments differ, but the encoded words do not.)

This is the **trust bridge** in practice: a hardware engineer can read `hll.s`
against a logic-analyzer capture, and a reviewer can prove the abstraction
changed no bytes.

> **Byte-identity is "modulo register allocation."** Spec §9 sets the success
> criterion as *byte-identical modulo register allocation*: identical bus
> behaviour and identical CRC bytes, with the exact `get_byte`/scratch register
> *numbers* coinciding only if the allocator reuses one scratch register for the
> discarded `recv` values the way the references reuse `t0`/`t1`/`t2`. That is an
> allocator-strategy detail, not a semantic one — the wire bytes are the same
> regardless. The read counts are fixed and unambiguous (decision D12): three
> `recv` targets for the peripheral read, `ndata = 0` for the OOB / VW / flash
> write completions. See the [read-accounting
> note](language-guide.md#a-note-on-read-accounting).
