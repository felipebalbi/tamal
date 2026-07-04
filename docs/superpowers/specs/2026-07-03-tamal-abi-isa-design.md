# Tamal — `tamal-abi::isa` Rust ISA Encoding Design

Date: 2026-07-03
Status: Approved (design); implementation not started
Scope: The Rust port of the tamal instruction encoding into `crates/tamal-abi` —
a **byte-for-byte mirror** of the HDL's `Tamal.Isa` (`Instr` ADT + total
`encode`/`decode` + `DecodeError`) and its peer `Tamal.Config` (the `SET_CONFIG`
payload codec). This is the ABI foundation the assembler emits through; the
`tamal-asm` lexer/parser/pseudo-op layer is a **separate, later spec**.

Companion to the ISA & HDL Engine design
(`docs/superpowers/specs/2026-07-01-tamal-isa-design.md`, esp. §4 — the 32-bit
word layout — §5/§6 — the opcode tables — and §7.2 — the `SET_CONFIG` fields),
the ALU/branch design (`.../2026-07-01-tamal-alu-branch-design.md`, esp. §9 — the
reserved-`SHIFT`-op decode tightening), and the wire-format design
(`.../2026-07-02-tamal-wire-format-design.md`, esp. §7 — little-endian word
packing). The authoritative reference implementations are
`hdl/src/Tamal/Isa.hs` and `hdl/src/Tamal/Config.hs`.

---

## 1. Purpose & the key reframing

`tamal-abi` is the project ABI: the transport-agnostic contract shared by the
host tooling and the FPGA gateware. Its `isa` module is currently a placeholder.
This spec fills it with a Rust encoding that is **provably identical** to what the
FPGA engine decodes, so that `tamal-asm` (next spec) can build `Instr` values and
emit tamal bytecode the engine accepts byte-for-byte.

The design principle is **faithful mirror, not reinvention**. The HDL
`Tamal.Isa.hs` is a complete, hedgehog-tested implementation: 36 `Instr`
constructors, a total `encode :: Instr -> BitVector 32`, and a total
`decode :: BitVector 32 -> Either DecodeError Instr` that enforces
reserved-must-be-zero. The Rust code ports it one-for-one; the same laws
(`decode ∘ encode ≡ Right`; "any word decodes canonical or traps") are asserted
on both sides, and cross-language golden vectors guard against drift.

A second, smaller mirror comes along: `Tamal.Config` (the 6-bit `SET_CONFIG`
payload codec). The `Instr::SetConfig` variant carries the **raw** 6-bit payload
(so non-v1 combos remain round-trippable); the typed `Config ↔ 6-bit` codec lives
in a sibling `config` module, which also gains the host-only `pack` direction the
gateware never needs (the host builds `SET_CONFIG` payloads; the engine only
decodes them).

### 1.1 The purity observation

An encode/decode library is **entirely pure** — no I/O, no state, no clock. There
is effectively no imperative shell to isolate here. The relevant discipline is
therefore decomposition: split the logic into small **total** functions (field
split/join, sub-field packers, the `only` reserved-field guard) so each is
independently unit-tested and each maps to a specific piece of the HDL for
line-by-line parity review. The imperative-shell concern proper belongs to the
later `tamal-asm` (file I/O, CLI) and `tamal-loader` (transport) crates.

## 2. Scope & non-goals

**In scope**

- `tamal_abi::isa`:
  - The `Instr` enum — 36 variants, 1:1 with `Tamal.Isa`.
  - Checked operand **newtypes** for non-native field widths (`Reg`, `Imm11`,
    `Imm20`, `Tar4`, `Cfg6`, `Sr5`, `Amt5`, `BitCount`, `WaitCond`,
    `WaitTimeout`) and the `ShiftOp` enum.
  - `DecodeError` (`ReservedFieldNonZero | OpcodeUnimplemented | IllegalOpcode`).
  - Total `Instr::encode(&self) -> u32` and total
    `Instr::decode(u32) -> Result<Instr, DecodeError>`.
  - `program_to_le_bytes(&[Instr]) -> Vec<u8>` (little-endian word packing).
- `tamal_abi::config`:
  - `Role`, `IoMode`, `Sck`, `AlertSource` enums, `Config`, `ConfigError`.
  - `decode_config(Cfg6) -> Result<Config, ConfigError>` (v1-strict mirror).
  - `Config::pack(&self) -> Cfg6` (host-only encode direction).
- proptest property tests + golden-vector tests proving parity with the HDL.

**Out of scope (deferred)**

- **The assembler** — lexer, parser, pseudo-op expansion (`li`, `j`, `nop`, `mv`,
  `beqz`/`bnez`, …), directives (`.text`/`.equ`/`.macro`/…), labels/local labels,
  the `set_config CONTROLLER, X1, …` symbolic-operand surface, and the CLI. Next
  spec (`tamal-asm`).
- **Textual disassembly** (`Instr` → mnemonic string). That is the assembler's
  surface, not the ABI's; `isa` provides only the structured `Instr` + `Debug`.
- **The control/result wire format** Rust mirror (COBS framing, CRC-8 over
  frames, `ControlMsg`/result codecs). Deferred post-silicon per the wire-format
  design; the `control`/`trace` placeholder modules in `lib.rs` stay untouched.
- **x16..x31 rejection.** `decode` deliberately does **not** reject register
  fields ≥ 16 (matching the HDL, where aliasing is a regfile concern and
  windowing is an assembler/engine concern). `Reg` therefore spans `0..=31`.
- **`SET_CONFIG` liveness policy at the ABI layer.** `decode_config` mirrors the
  engine's v1-strict acceptance, but whether to *reject* a non-v1 config at
  assemble time is a `tamal-asm` policy decision (§5.3).
- **`no_std`.** Host-side crate; stays `std`, keeps `#![forbid(unsafe_code)]`.

## 3. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | **Faithful 1:1 mirror of `Tamal.Isa`** — same 36 constructors, same field semantics, same `DecodeError` taxonomy. | The FPGA `decode` is the ground truth; a structurally identical Rust port makes byte-parity checkable function-by-function and keeps the two in sync as the ISA evolves. |
| D2 | **Checked newtypes for non-native widths ⇒ `encode` is total/infallible.** | Mirrors the HDL, where `BitVector n` guarantees width by type. Making illegal widths unrepresentable pushes range errors to construction (Rust-idiomatic "parse, don't validate") and keeps `encode` a pure total function like the Haskell. |
| D3 | **`ShiftOp` is an enum (`Sll`/`Srl`/`Sra`); the reserved `0b11` is unrepresentable.** | No constructible `Instr` can encode to a trapping SHIFT word, so the round-trip law holds without a special case. Matches the ALU/branch design §9 decode tightening. |
| D4 | **`Instr::SetConfig` holds the raw `Cfg6`, not a typed `Config`.** | The 6-bit space includes `io_mode=0b11` and non-v1 combos that must round-trip; a typed `Config` cannot represent them, which would break `decode ∘ encode ≡ id`. Exactly the HDL split (`SetConfig (BitVector 6)` + separate `Tamal.Config`). |
| D5 | **`decode` does not reject `Reg ≥ 16`; `Reg` spans `0..=31`.** | Byte-parity with the HDL `decode`, which passes the raw 5-bit field through (`Reg = BitVector 5`). Windowing to x0..x15 is an assembler/engine responsibility. |
| D6 | **`config` module also gains a host-only `Config::pack`.** | The gateware only ever *decodes* `SET_CONFIG`; the host must *build* the payload. `pack` is the encode direction the HDL doesn't need. The bit layout stays centralized in the ABI, shared with the HDL. |
| D7 | **Small declarative macro `bounded!(Name, repr, BITS)` generates the newtypes.** | One pure, unit-tested block instead of a dozen near-identical `new`/`TryFrom`/`bits` impls. |
| D8 | **In-crate `#[cfg(test)]` tests (proptest + golden), not a `tests/` integration dir.** | The pure sub-blocks worth testing (`split_word`, the packers) are `pub(crate)`; inline tests exercise the private seam directly. Property tests mirror `hdl/tests/Test/Isa.hs`. |
| D9 | **Golden encode vectors cross-checked against the HDL** (a documented `stack` snippet regenerates them). | Turns "hand-computed and hopefully right" into "provably identical to the gateware," the single strongest guard against silent Rust↔Haskell drift. |
| D10 | **TDD throughout, proptest where a law generalizes.** | Matches the repo's hedgehog baseline. Round-trip and canonical-or-traps are universally-quantified laws (proptest); reserved-field traps and golden layouts are specific vectors (unit). |

## 4. The `isa` module

### 4.1 Word layout (spec §4, `Tamal.Isa` `splitWord`/`joinW`)

```
 31 30 | 29 .. 26 | 25 .. 21 | 20 .. 16 | 15 .. 11 | 10 .. 0
 group |   sub    |    rd    |   rs1    |   rs2    |   imm
  (2)  |   (4)    |   (5)    |   (5)    |   (5)    |  (11)
```

`group`: `0b00`=BUS, `0b01`=CTRL, `0b10`=DATA, `0b11`=reserved (⇒ `IllegalOpcode`).

### 4.2 Operand newtypes

Every field whose width is not a native Rust integer gets a bounded newtype,
generated by `bounded!(Name, repr, BITS)`. Each exposes `new(v) -> Option<Self>`
(checked), `impl TryFrom<repr>`, `bits(&self) -> repr`, and a **private
infallible** `from_bits(masked) -> Self` used by `decode` (fields are already
width-masked by `split_word`).

| Newtype | Repr | Valid range | Used by |
|---|---|---|---|
| `Reg` | `u8` | `0..=31` (5-bit; **not** windowed) | every `rd`/`rs1`/`rs2` |
| `Imm11` | `u16` | 11-bit (`0..=0x7FF`) | branch offset, `LoadImm`, `Addi`/`Andi`/`Ori`/`Xori`, `Mark` label |
| `Imm20` | `u32` | 20-bit (`0..=0xF_FFFF`) | `Lui` |
| `Tar4` | `u8` | 4-bit (`0..=0xF`) | `TarImm` |
| `Cfg6` | `u8` | 6-bit (`0..=0x3F`) | `SetConfig` payload |
| `Sr5` | `u8` | 5-bit (`0..=0x1F`) | `Rdsr` |
| `Amt5` | `u8` | 5-bit (`0..=0x1F`) | `Shift` amount |
| `BitCount` | `u8` | **`1..=8`** (semantic; encodes `n-1`) | `PutBitsImm`/`PutBitsReg`/`GetBits` |
| `WaitCond` | `u8` | 2-bit (`0..=3`) | `WaitOn` |
| `WaitTimeout` | `u16` | 9-bit (`0..=0x1FF`) | `WaitOn` |

`ShiftOp` is a hand-written enum, not a `bounded!`:

```rust
pub enum ShiftOp { Sll, Srl, Sra }   // encodes 0b00 / 0b01 / 0b10; 0b11 unrepresentable
```

`BitCount` is the one newtype whose value is **not** the stored bit pattern: it
holds the semantic count `1..=8` and `encode`/`decode` apply the `n-1` offset
(the HDL `Index 8` stores `n-1` directly). This keeps the `Instr` surface
readable (`PutBitsImm(BitCount::new(8)?, 0xAB)`) while matching the wire bits.

### 4.3 The `Instr` enum (36 variants)

Clean Rust variant names (`And`/`Or`/`Xor`, no trailing underscore); exact-8-bit
fields stay plain `u8`.

```rust
pub enum Instr {
    // BUS group (00)
    CsAssert,
    CsDeassert,
    PutByteImm(u8),
    PutByteReg(Reg),
    GetByte(Reg),
    PutBitsImm(BitCount, u8),
    PutBitsReg(Reg, BitCount),
    GetBits(Reg, BitCount),
    TarImm(Tar4),
    TarReg(Reg),
    RstAssert,
    RstDeassert,
    GetAlert(Reg),
    // CTRL group (01)
    Halt(u8),
    Beq(Reg, Reg, Imm11),
    Bne(Reg, Reg, Imm11),
    Bltu(Reg, Reg, Imm11),
    Bgeu(Reg, Reg, Imm11),
    WaitOn(Reg, WaitCond, WaitTimeout),
    SetConfig(Cfg6),
    Mark(Imm11, Reg),
    CrcReset,
    // DATA group (10)
    LoadImm(Reg, Imm11),
    Lui(Reg, Imm20),
    Mov(Reg, Reg),
    Add(Reg, Reg, Reg),
    Addi(Reg, Reg, Imm11),
    Sub(Reg, Reg, Reg),
    And(Reg, Reg, Reg),
    Andi(Reg, Reg, Imm11),
    Or(Reg, Reg, Reg),
    Ori(Reg, Reg, Imm11),
    Xor(Reg, Reg, Reg),
    Xori(Reg, Reg, Imm11),
    Shift(Reg, Reg, ShiftOp, Amt5),
    Rdsr(Reg, Sr5),
}
```

Derives: `Debug, Clone, Copy, PartialEq, Eq`. `DecodeError` derives the same plus
a `thiserror::Error` `Display`:

```rust
pub enum DecodeError {
    ReservedFieldNonZero,   // a reserved field held a non-zero value
    OpcodeUnimplemented,    // recognised but unimplemented (defined for parity; unproduced today)
    IllegalOpcode,          // group 0b11, or an unknown sub-opcode
}
```

`OpcodeUnimplemented` mirrors the HDL variant, which is defined but not currently
produced by `decode`. It is kept so the `DecodeError` type matches the gateware's
one-for-one; if a future decoder path needs it, both sides already carry it.

### 4.4 Pure decomposition of `encode`/`decode`

Every function below is total and pure; the decomposition tracks `Isa.hs` so a
reviewer can diff Rust against Haskell arm-by-arm.

- **Field seam** (`pub(crate)`): `split_word(u32) -> Fields` and
  `join_word(Fields) -> u32`, where
  `Fields = (group: u8, sub: u8, rd: u8, rs1: u8, rs2: u8, imm: u16)`. These are
  the Rust analog of the HDL `bitCoerce`, with the fixed shifts/masks of §4.1.
  `join_word(split_word(w)) == w` for all `w`.
- **Sub-field packers** (`pub(crate)`, each with a pack/unpack pair):
  - `bits_imm(n_minus_1: u8, byte: u8) -> u16` / inverse — `imm[10:8]=n-1`,
    `imm[7:0]=byte` (the `PUT_BITS`/`GET_BITS` layout).
  - `split_imm20(i20) -> (rs1, rs2, imm)` / `join_imm20(rs1, rs2, imm) -> (hi, i20)`
    — LUI's `0 ++ i20` spread across `rs1 ++ rs2 ++ imm`; `hi` is the reserved
    bit 20.
  - `wait_pack(cond, timeout) -> u16` / inverse — `imm = cond:2 ++ timeout:9`.
  - `shift_pack(op, amt) -> u16` / `shift_unpack(imm) -> (op, mid, amt)` —
    `imm = op:2 ++ 0:4 ++ amt:5`; `mid` is the 4 reserved bits.
- **`Instr::encode(&self) -> u32`** — one match arm per variant, composing
  `join_word` with the packers. Total, infallible, the exact inverse of a
  successful `decode`.
- **`Instr::decode(u32) -> Result<Instr, DecodeError>`** — `split_word`, dispatch
  on `group` (`0b11 ⇒ IllegalOpcode`), then `decode_bus`/`decode_ctrl`/
  `decode_data`. Each arm rebuilds the instruction and gates acceptance through:

  ```rust
  fn only(all_reserved_zero: bool, i: Instr) -> Result<Instr, DecodeError> {
      if all_reserved_zero { Ok(i) } else { Err(DecodeError::ReservedFieldNonZero) }
  }
  ```

  An unknown sub-opcode ⇒ `IllegalOpcode`; a non-zero reserved field ⇒
  `ReservedFieldNonZero`; a SHIFT `op == 0b11` ⇒ `ReservedFieldNonZero` (the
  `shift_unpack` `mid`/`op` guards mirror `decodeData` sub `0xc`). `decode` never
  inspects the register window.

### 4.5 Reserved-field guards (mirror of `Isa.hs`)

The per-arm "all reserved fields zero" predicate is transcribed directly from the
HDL; the notable ones:

- `PutByteImm`: `rd==0 && rs1==0 && rs2==0 && imm[10:8]==0`.
- `TarImm`: `rd==0 && rs1==0 && rs2==0 && imm[10:4]==0`.
- `Halt`: `rd==0 && rs1==0 && rs2==0 && imm[10:8]==0`.
- `SetConfig`: `rd==0 && rs1==0 && rs2==0 && imm[10:6]==0`.
- `Lui`: the reserved bit 20 (`hi`) is zero.
- `Shift`: `rs2==0 && mid==0 && op != 0b11`.
- `Rdsr`: `rs1==0 && rs2==0 && imm[10:5]==0`.
- Branch ops (`Beq`/`Bne`/`Bltu`/`Bgeu`): `rd==0`.
- `Mark`: `rd==0 && rs2==0`.
- No-operand ops (`CsAssert`, `CsDeassert`, `RstAssert`, `RstDeassert`,
  `CrcReset`): all of `rd`/`rs1`/`rs2`/`imm` zero.

### 4.6 Little-endian program helper (spec §4, wire §7)

```rust
pub fn program_to_le_bytes(program: &[Instr]) -> Vec<u8>
```

Encodes each `Instr` and appends its `u32::to_le_bytes`, so the wire byte order
(ISA §4 — little-endian words; wire §7 — `0xAABBCCDD → [DD, CC, BB, AA]`) lives in
the ABI rather than being re-derived by the assembler and the loader. A future
`tamal-asm` writes this to a bytecode file; a future `tamal-loader` wraps it in a
`LOAD_PROGRAM` frame.

## 5. The `config` module

### 5.1 Types (mirror of `Tamal.Config`)

```rust
pub enum Role        { Controller, Target }          // 1 bit: 0, 1
pub enum IoMode      { X1, X2, X4 }                   // 2 bits: 0b00, 0b01, 0b10
pub enum Sck         { Sck20, Sck33, Sck50, Sck66 }   // 2 bits: 0b00..0b11
pub enum AlertSource { AlertPin, AlertIo1 }           // 1 bit: 0, 1

pub struct Config {
    pub role: Role,
    pub io_mode: IoMode,
    pub sck: Sck,
    pub alert_source: AlertSource,
}

pub enum ConfigError { UnsupportedRole, UnsupportedIoMode, UnsupportedSck }
```

### 5.2 Payload layout (from `decodeConfig`'s `bitCoerce`)

```
 Cfg6:  [5] = role   [4:3] = io_mode   [2:1] = sck   [0] = alert_source
```

### 5.3 Codec directions

- `Config::pack(&self) -> Cfg6` — **total** host-only encode. Builds the 6 bits
  from the enums per §5.2. This is what `tamal-asm` uses to lower
  `set_config CONTROLLER, X1, SCK20, ALERT_PIN` into `Instr::SetConfig(cfg6)`.
- `decode_config(Cfg6) -> Result<Config, ConfigError>` — the **v1-strict** mirror
  of the HDL: accepts only `(Controller, X1, Sck20, *)`; otherwise, in the HDL's
  priority order, role mismatch ⇒ `UnsupportedRole`, else non-X1 ⇒
  `UnsupportedIoMode`, else ⇒ `UnsupportedSck`.

`pack` and `decode_config` are intentionally **not** general inverses
(`decode_config` rejects non-v1 selections), which is precisely why
`Instr::SetConfig` holds the raw `Cfg6` (§3 D4). Policy — whether the assembler
*rejects* a non-v1 config at assemble time (e.g. by round-tripping the packed
payload through `decode_config` and surfacing the `ConfigError`) or emits it
deliberately — is a `tamal-asm` decision, out of scope here. The ABI provides
both directions; it does not impose the policy.

## 6. Testing (TDD, proptest + golden vectors)

In-crate `#[cfg(test)]` modules. Property tests use `proptest` and mirror
`hdl/tests/Test/Isa.hs`; golden vectors are cross-checked against the HDL (§6.3).
Red → green → refactor order (this is the skeleton the implementation plan will
expand into tasks):

### 6.1 `isa` tests

1. **Field seam** — `join_word(split_word(w)) == w` (proptest over `w`), plus a
   golden vector: a word with distinct field values decomposes to the exact
   tuple, pinning bit positions.
2. **Newtypes** — one generic proptest per `bounded!` type: `new(v)` is `Some`
   iff `v` is in range and `None` otherwise; `from_bits(x.bits()) == x`.
   `BitCount` additionally: `new(0)` and `new(9)` are `None`; `new(1..=8)` map to
   stored `0..=7`.
3. **Sub-field packers** — pack/unpack round-trips (proptest) for `bits_imm`,
   `imm20`, `wait`, `shift`, plus golden vectors for the LUI spread and the SHIFT
   `op:2 ++ 0:4 ++ amt:5` layout.
4. **Golden encode table** — a checked-in `&[(Instr, u32)]` of representative
   instructions across all three groups (including the tricky `Lui`, `WaitOn`,
   `Shift`, `PutBitsImm`), asserting `instr.encode() == word`. Cross-checked
   against the HDL (§6.3).
5. **Decode traps (unit)** — `decode(cs_assert_word + 1) == Err(ReservedFieldNonZero)`;
   a raw SHIFT word with `op == 0b11` ⇒ `Err(ReservedFieldNonZero)`; a
   `group == 0b11` word ⇒ `Err(IllegalOpcode)`; an unknown sub-opcode ⇒
   `Err(IllegalOpcode)`.
6. **Round-trip law** — proptest strategies `arb_reg` (`0..=31`), `arb_imm11`,
   `arb_imm20`, …, and `arb_bus_instr`/`arb_ctrl_instr`/`arb_data_instr`/
   `arb_instr`: `decode(i.encode()) == Ok(i)`.
7. **Canonical-or-traps law** — for any `u32 w`: `decode(w)` is `Err(_)`, or
   `Ok(i)` with `i.encode() == w` (the HDL's "any word decodes canonical or
   traps" — catches a too-lax reserved-field check in any group).
8. **LE helper** — `program_to_le_bytes` golden: `[PutByteImm(0x64)]` →
   `[0x64, 0x00, 0x00, 0x08]` (word `0x0800_0064`: `sub=0x2` at bits `[29:26]`,
   `imm=0x64`; serialized LE).

### 6.2 `config` tests

- `decode_config` golden: the one accepted payload → the expected `Config`; one
  payload per rejection class → its `ConfigError` (role bit set ⇒
  `UnsupportedRole`; io≠`0b00` ⇒ `UnsupportedIoMode`; sck≠`0b00` with role/io v1 ⇒
  `UnsupportedSck`).
- `pack` golden: `Config { Controller, X1, Sck20, AlertPin }.pack() == 0x00`;
  flipping each field flips the pinned bits (`AlertIo1` ⇒ byte `0x01`; `Sck33`,
  the 2-bit field value `0b01` placed in `[2:1]`, ⇒ byte `0x02`; `X2`, field
  `0b01` in `[4:3]`, ⇒ byte `0x08`; `Target` ⇒ byte `0x20`).
- Round-trip for the v1 config: `decode_config(c.pack()) == Ok(c)` for both
  alert sources.

### 6.3 Cross-language parity (the drift guard)

The golden encode table (§6.1.4) is not hand-computed and trusted; it is
regenerated/verified from the HDL. The spec records the exact snippet so a future
maintainer can re-derive it after any ISA change, from `hdl/`:

```haskell
-- ghci: stack ghci src/Tamal/Isa.hs
--   mapM_ (print . encode) [CsAssert, PutByteImm 0x64, TarImm 2, GetByte 5, ...]
-- Each printed BitVector 32 is the u32 the Rust golden must match.
```

The same instruction list is kept in both the Haskell dump command (in this spec)
and the Rust golden table, so a divergence in either `encode` fails the Rust test
against a value provably produced by the gateware.

## 7. Dependencies & crate wiring

- **workspace `Cargo.toml`** — add `proptest = "1"` to `[workspace.dependencies]`.
- **`crates/tamal-abi/Cargo.toml`**:
  - `[dependencies]`: `thiserror.workspace = true`.
  - `[dev-dependencies]`: `proptest.workspace = true`.
  - Stays MIT, `std`, `#![forbid(unsafe_code)]`.
- **`crates/tamal-abi/src/lib.rs`** — replace the inline placeholder `pub mod isa
  { … }` with `pub mod isa;` and add `pub mod config;`. Leave the `control` and
  `trace` placeholder modules unchanged (deferred to the wire-format Rust mirror).
- **No other crate changes.** `tamal-asm` consumes this in the next spec.

### 7.1 Files touched

```
new:      crates/tamal-abi/src/isa.rs      -- Instr, newtypes, ShiftOp, DecodeError,
                                           --   encode/decode, program_to_le_bytes, tests
          crates/tamal-abi/src/config.rs   -- Role/IoMode/Sck/AlertSource, Config,
                                           --   ConfigError, decode_config, pack, tests

modified: crates/tamal-abi/src/lib.rs      -- pub mod isa; pub mod config;
          crates/tamal-abi/Cargo.toml      -- thiserror dep + proptest dev-dep
          Cargo.toml                       -- proptest in [workspace.dependencies]
```

## 8. Verification

From the repo root:

```
cargo build -p tamal-abi
cargo test  -p tamal-abi          # proptest round-trip/canonical + golden vectors
cargo clippy -p tamal-abi --all-targets -- -D warnings
cargo fmt --all --check
```

The existing CI rust job (`build`/`test`/`clippy -D warnings`/`fmt`) covers the
crate with no workflow change; proptest runs under `cargo test`.

## 9. Out of scope / follow-ups (roadmap)

1. **This spec** — `tamal-abi::isa` + `tamal-abi::config` (pure, proptest). ←
   implement next.
2. **`tamal-asm`** — the RISC-V-flavored assembler on top of this ABI: lexer,
   parser, pseudo-op expansion, directives, labels/local labels, the symbolic
   `set_config`/`rdsr` operand surface, `li` constant-tiling for the ISA §7.3
   reachability gap, diagnostics, and the CLI. Consumes `Instr`/`encode`/
   `Config::pack`. Separate spec.
3. **`tamal-abi` wire-format mirror** — the Rust port of the COBS/CRC-8 control
   and result framing (`Tamal.Wire`), deferred post-silicon per the wire-format
   design; pairs with the `tamal-loader` transport.

## 10. Prior art & references

- **`hdl/src/Tamal/Isa.hs`** — the authoritative encode/decode this crate mirrors.
- **`hdl/src/Tamal/Config.hs`** — the authoritative `SET_CONFIG` codec.
- **`hdl/tests/Test/Isa.hs`** — the property/vector suite the Rust tests mirror.
- **[mole](https://github.com/felipebalbi/mole)** — the sibling I2C/I3C rig whose
  transport-agnostic ABI split (bytecode down, semantics in the host) motivates
  keeping the encoding in a shared, dependency-light `tamal-abi`.
