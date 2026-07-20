# The `espi` standard library

> **Status:** designed, not yet implemented. The `espi` module below is the v1
> surface as specified in
> [`docs/superpowers/specs/2026-07-20-tamal-lang-design.md`](../superpowers/specs/2026-07-20-tamal-lang-design.md)
> §4.3. Its exact final contents and the search-path precedence are spec open
> question §8.4.

The tamal engine has **zero eSPI knowledge**, and so does the language core. All
the eSPI vocabulary — command opcodes, cycle types, verdict codes, the
poll/verify skeleton — lives here, in an importable, **source-form** library you
can read, fork, and extend. `import espi` works out of the box; the library ships
vendored with the toolchain and is discovered on a search path.

Keeping it plain source is deliberate. A hardware engineer can open `espi.tam`
and see that `PUT_OOB` is `0x06` and that `command` is just framing plus a poll
plus a residue check — no magic, no compiled blob. If a helper does not fit your
test, you copy it and change it.

---

## The bundled `espi.tam` (excerpt)

This is the block that was copy-pasted into every hand-written example `.s`,
captured once:

```rust
// espi.tam — bundled eSPI library (excerpt; source-form, forkable)
pub const PUT_IORD1 = 0x44
pub const PUT_OOB   = 0x06

pub enum Verdict: byte { Ok = 0x00, Crc = 0x11 }

pub proc controller() { config controller, x1, sck20, alert_pin }

// Send a host-built packet, append its CRC-8, turn the bus around, poll past
// WAIT_STATE, read `ndata` payload + 2 status bytes, consume the trailing CRC
// byte and verify the residue. The block copy-pasted into every example .s.
pub proc command(pkt: bytes, ndata: int, err: byte = Verdict.Crc) {
    frame {
        send pkt + crc8                 // TX CRC-8 auto-appended over pkt
        tar 2
        wait_state
        repeat ndata { recv _ }         // payload bytes (compile-time unroll)
        recv status0, status1
        expect crc else err             // consumes trailing CRC byte, checks residue
    }
}

// Pure value-returning helper: build a short-I/O header from an address.
pub fn iowr_hdr(op: byte, addr: int) -> bytes { [op, hi(addr), lo(addr)] }
```

Four public items, each pulling its weight:

| Item | Kind | What it gives you |
|---|---|---|
| `PUT_IORD1`, `PUT_OOB` | `const byte` | named eSPI command opcodes (`0x44`, `0x06`) |
| `Verdict` | `enum` | named verdict codes: `Verdict.Ok = 0x00`, `Verdict.Crc = 0x11` |
| `controller()` | `proc` | the standard v1 controller configuration |
| `command(pkt, ndata, err)` | `proc` | the whole send → turnaround → poll → verify skeleton |
| `iowr_hdr(op, addr)` | `fn` | build a short-I/O header `[op, hi(addr), lo(addr)]` |

Everything is `pub`, so all of it is reachable through `import espi`. A real
`espi.tam` will also carry the other channels' opcodes (`PUT_VWIRE = 0x04`,
`PUT_FLASH_C = 0x08`, cycle types, response codes); the excerpt shows the shape.

---

## Importing and calling

```rust
import espi

test t {
    espi.controller()                       // a proc call: config the engine
    let hdr = espi.iowr_hdr(0x44, 0x0064)   // a fn call: build [0x44, 0x00, 0x64]
    // …
    fail espi.Verdict.Crc                   // an enum member: halt 0x11
}
```

Everything is **qualified** by the module name `espi`:

- `espi.PUT_OOB` — a constant.
- `espi.Verdict.Crc` — an enum member (`module.Enum.Member`).
- `espi.controller()` — a `proc`; inlines its body at the call site.
- `espi.command(...)` — a `proc`; inlines the framing/poll/verify skeleton.
- `espi.iowr_hdr(...)` — a `fn`; replaced by its returned `bytes` value.

`proc` calls **emit** instructions (inlined, with fresh registers); `fn` calls
**return** a compile-time value. Neither is a runtime call — the ISA has no
`call`/`ret`. See the [language guide §7](language-guide.md#7-proc-and-fn).

---

## A complete test built from the library

This is the OOB (Out-Of-Band) channel example: it tunnels a short SMBus message
over `PUT_OOB` and verifies the ACCEPT completion's CRC. The 62-line
[`examples/oob_smbus_msg.s`](../../examples/oob_smbus_msg.s) collapses to about
ten meaningful lines, and its hand-written `0xB1` CRC becomes `+ crc8` inside the
one reviewed `command` proc.

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

Named arguments (`pkt = …`, `ndata = …`) make the call self-documenting: you can
see at the call site that the payload is one thing and the count another, without
opening `command`'s definition.

### What it lowers to

The `command` proc inlines; the packet's CRC (`0xB1`) is folded at compile time;
the poll/residue/verdict skeleton expands. The result is the transaction in
`oob_smbus_msg.s`:

```asm
        .globl _start
_start:
        set_config controller, x1, sck20, alert_pin   # espi.controller()
        cs_assert                                      # command → frame {
        put_byte 0x06                                  #   send [PUT_OOB,
        put_byte 0x21                                  #         0x21,
        put_byte 0x00                                  #         0x00,
        put_byte 0x04                                  #         0x04,
        put_byte 0x10                                  #         0x10,
        put_byte 0x00                                  #         0x00,
        put_byte 0x01                                  #         0x01,
        put_byte 0xAB                                  #         0xAB]
        put_byte 0xB1                                  #   + crc8 = crc8(pkt)
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
        cs_deassert                                    # } frame exit: CS# high before verdict
        bnez x7, .Lfail0                               #   expect crc else Verdict.Crc (0x11)
        halt 0x00                                      # pass
.Lfail0:
        halt 0x11
```

The `0xB1` is `crc8([0x06, 0x21, 0x00, 0x04, 0x10, 0x00, 0x01, 0xAB])`, computed
by the same code that runs on the wire. Change any payload byte and it updates.

> **Choosing `ndata`.** `ndata` is the count of **payload** bytes the completion
> returns — not counting the response code or the status/CRC bytes. An OOB *write*
> completion carries no payload, so `ndata = 0`: `repeat ndata { recv _ }`
> contributes zero reads, and the response phase is just the poll, two status
> bytes, and the CRC (exactly what the lowering above shows). A *read* that
> returns data — like `peripheral_io_read`, with one I/O byte — uses `ndata = 1`.
> The rule follows from how the reads line up: `wait_state` consumes the response
> code, `command` reads `ndata` payload bytes then the two status bytes, and
> `expect crc` consumes the trailing CRC. See [language guide → read
> accounting](language-guide.md#a-note-on-read-accounting).

---

## Forking the library

Because `espi` is plain source, adapting it is a copy and an edit — no build
system, no version negotiation.

1. Copy the vendored `espi.tam` (or just the proc you need) into your project,
   say `lib/espi_myrig.tam`.
2. Edit it — add an opcode, tweak the poll, add a channel helper.
3. Import your fork by path instead of the bundled module:

```rust
import "lib/espi_myrig.tam" as espi     // same call sites, your implementation
```

Because references are qualified (`espi.command(...)`), aliasing your fork to the
name `espi` means the rest of the test is unchanged. You have swapped the
implementation without touching the call sites.

When you write your own library from scratch, mark the public surface with `pub`
and keep helpers private — see [cookbook → writing your own
library](cookbook.md#recipe-7-write-your-own-library) for a worked example, and
[language guide §12](language-guide.md#12-imports-and-modules) for the module and
resolution rules.
