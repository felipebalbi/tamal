# Tamal `tamal-abi::isa` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fill `crates/tamal-abi` with a Rust port of the FPGA ISA encoding — the `Instr` type, checked operand newtypes, total `encode`/`decode`, and the `SET_CONFIG` codec — that is byte-for-byte identical to the HDL `Tamal.Isa`/`Tamal.Config`.

**Architecture:** Two new pure modules, `tamal_abi::isa` and `tamal_abi::config`. `isa` decomposes into small total functions: a `bounded!` macro that generates width-checked operand newtypes, a `split_word`/`join_word` field seam, four sub-field packers, a 36-variant `Instr` enum, total `Instr::encode`/`Instr::decode`, and a little-endian program helper. `config` mirrors `Tamal.Config` and adds a host-only `pack` direction. Everything is `#[forbid(unsafe_code)]`, pure, and TDD-built with proptest laws + HDL-cross-checked golden vectors.

**Tech Stack:** Rust (edition 2024, rustc ≥ 1.85), Cargo workspace; `thiserror` (error `Display`); `proptest` (property tests, dev-dependency). Source of truth: `docs/superpowers/specs/2026-07-03-tamal-abi-isa-design.md`, and the reference implementations `hdl/src/Tamal/Isa.hs` + `hdl/src/Tamal/Config.hs`.

## Global Constraints

- Edition `2024`, `rust-version = "1.85"` (workspace-inherited); do not add a per-crate `edition`/`rust-version`.
- `#![forbid(unsafe_code)]` stays at the top of `crates/tamal-abi/src/lib.rs`.
- The crate is MIT (workspace `license`); no SPDX headers in Rust files (those are for `hdl/**/*.hs` only).
- `encode` is **total and infallible**; range validation lives in newtype `new` constructors. `decode` is total: every `u32` yields `Ok(Instr)` or `Err(DecodeError)`.
- `decode` must **not** reject register fields ≥ 16; `Reg` spans `0..=31`.
- Byte order on the wire is **little-endian** (`0xAABBCCDD → [DD, CC, BB, AA]`).
- Must pass `cargo clippy -p tamal-abi --all-targets -- -D warnings` and `cargo fmt --all --check`. In particular: do **not** add a `TryFrom<_> -> Result<_, ()>` (triggers `clippy::result_unit_err`); newtypes expose `new` returning `Option`, not `TryFrom`.
- Word layout (all tasks): `group[31:30] · sub[29:26] · rd[25:21] · rs1[20:16] · rs2[15:11] · imm[10:0]`. Groups: `0b00`=BUS, `0b01`=CTRL, `0b10`=DATA, `0b11`=reserved.

---

## File Structure

**New files**

| File | Responsibility |
|------|----------------|
| `crates/tamal-abi/src/isa.rs` | `bounded!` macro, operand newtypes, `ShiftOp`, `Fields`/`split_word`/`join_word`, sub-field packers, `Instr`, `DecodeError`, `encode`/`decode`, `program_to_le_bytes`, and their `#[cfg(test)]` tests |
| `crates/tamal-abi/src/config.rs` | `Role`/`IoMode`/`Sck`/`AlertSource`, `Config`, `ConfigError`, `decode_config`, `Config::pack`, and tests |

**Modified files**

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | add `proptest = "1"` to `[workspace.dependencies]` |
| `crates/tamal-abi/Cargo.toml` | `[dependencies] thiserror.workspace = true`; `[dev-dependencies] proptest.workspace = true` |
| `crates/tamal-abi/src/lib.rs` | replace inline placeholder `pub mod isa { … }` with `pub mod isa;`; add `pub mod config;` |

**No change**

- `crates/tamal-abi/src/lib.rs` `control`/`trace` placeholder modules — stay as-is (deferred to the wire-format Rust mirror).

**Notes**

- `split_word`, `join_word`, the packers, and every newtype's `from_bits` are `pub(crate)` — tests and the `config` module use them; the public API is `Instr`, `DecodeError`, the newtypes (via `new`/`bits`/`count`), `program_to_le_bytes`, and the `config` items.
- Golden encode vectors are hand-derived in this plan and cross-checked against the HDL with the ghci snippet in Task 8 (spec §6.3).

---

## Task 1: Crate wiring + `bounded!` macro + width newtypes

Stand up dependencies, point `lib.rs` at a real `isa` module, and land the macro-generated width-checked newtypes (`Reg`, `Imm11`, `Imm20`, `Tar4`, `Cfg6`, `Sr5`, `Amt5`, `WaitCond`, `WaitTimeout`).

**Files:**
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/tamal-abi/Cargo.toml`
- Modify: `crates/tamal-abi/src/lib.rs`
- Create: `crates/tamal-abi/src/isa.rs`

**Interfaces:**
- Produces: `bounded!` macro; newtypes `Reg`, `Imm11`, `Imm20`, `Tar4`, `Cfg6`, `Sr5`, `Amt5`, `WaitCond`, `WaitTimeout`, each with `pub const fn new(v) -> Option<Self>`, `pub const fn bits(self) -> repr`, `pub const MAX: repr`, and `pub(crate) const fn from_bits(v) -> Self` (masks to width).

- [ ] **Step 1: Add `proptest` to workspace dependencies**

In `Cargo.toml` under `[workspace.dependencies]`, after the `serialport = "4"` line, add:

```toml
proptest = "1"
```

- [ ] **Step 2: Wire the crate's dependencies**

Replace the `[dependencies]` section of `crates/tamal-abi/Cargo.toml` (currently empty) so the file ends with:

```toml
[dependencies]
thiserror.workspace = true

[dev-dependencies]
proptest.workspace = true
```

- [ ] **Step 3: Point `lib.rs` at the real `isa` module**

In `crates/tamal-abi/src/lib.rs`, replace the entire inline `isa` module (the `pub mod isa { … }` block including its doc comment and placeholder body) with a single declaration line, keeping the doc comment:

```rust
/// The tamal instruction encoding.
///
/// The tamal engine's ISA is **inspired by — but not 100% compatible with — the
/// RISC-V 32-bit (RV32I) ISA**. This module is a byte-for-byte Rust mirror of the
/// HDL `Tamal.Isa`: the [`isa::Instr`] type, checked operand newtypes, and total
/// [`isa::Instr::encode`]/[`isa::Instr::decode`].
pub mod isa;
```

Leave the `control` and `trace` modules unchanged.

- [ ] **Step 4: Write the failing test — newtype range behavior**

Create `crates/tamal-abi/src/isa.rs` with only the test module (no macro yet, so it fails to compile):

```rust
//! Rust mirror of the HDL `Tamal.Isa` instruction encoding.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newtype_new_accepts_in_range_rejects_out_of_range() {
        assert_eq!(Reg::new(31).map(|r| r.bits()), Some(31));
        assert_eq!(Reg::new(32), None);
        assert_eq!(Imm11::new(0x7FF).map(|i| i.bits()), Some(0x7FF));
        assert_eq!(Imm11::new(0x800), None);
        assert_eq!(Imm20::new(0xF_FFFF).map(|i| i.bits()), Some(0xF_FFFF));
        assert_eq!(Imm20::new(0x10_0000), None);
        assert_eq!(Tar4::new(15).map(|t| t.bits()), Some(15));
        assert_eq!(Tar4::new(16), None);
        assert_eq!(Cfg6::new(0x3F).map(|c| c.bits()), Some(0x3F));
        assert_eq!(Cfg6::new(0x40), None);
        assert_eq!(WaitTimeout::new(0x1FF).map(|t| t.bits()), Some(0x1FF));
        assert_eq!(WaitTimeout::new(0x200), None);
    }

    #[test]
    fn from_bits_round_trips_masked_values() {
        assert_eq!(Reg::from_bits(5).bits(), 5);
        assert_eq!(Amt5::from_bits(0x1F).bits(), 0x1F);
        assert_eq!(WaitCond::from_bits(0x3).bits(), 0x3);
        assert_eq!(Sr5::from_bits(0).bits(), 0);
    }
}
```

- [ ] **Step 5: Run the test to verify it fails**

Run: `cargo test -p tamal-abi`
Expected: FAIL — compile error, `cannot find type Reg in this scope`.

- [ ] **Step 6: Implement the macro and newtypes**

At the top of `crates/tamal-abi/src/isa.rs` (above the `#[cfg(test)]` module), add:

```rust
/// Generate a width-checked operand newtype over `$repr` holding `$bits` bits.
///
/// `new` validates the range and returns `Option`; `from_bits` masks (infallible,
/// for `decode`, whose fields are already width-limited); `bits` reads the raw
/// value. Kept crate-private constructors (`from_bits`) so `decode` and `config`
/// can build values without re-checking.
macro_rules! bounded {
    ($(#[$doc:meta])* $name:ident, $repr:ty, $bits:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($repr);

        impl $name {
            /// The largest representable value (`2^BITS - 1`).
            pub const MAX: $repr = (1 << $bits) - 1;

            /// Construct if `v` fits in the field width, else `None`.
            pub const fn new(v: $repr) -> Option<Self> {
                if v <= Self::MAX { Some(Self(v)) } else { None }
            }

            /// The raw field value.
            pub const fn bits(self) -> $repr {
                self.0
            }

            /// Construct from an already width-limited field value (masks defensively).
            pub(crate) const fn from_bits(v: $repr) -> Self {
                Self(v & Self::MAX)
            }
        }
    };
}

bounded!(
    /// A 5-bit register selector, `x0..=x31`. `decode` does not window this to
    /// `x0..=x15`; that is an assembler/engine concern.
    Reg, u8, 5
);
bounded!(/// An 11-bit immediate / branch offset / label field.
    Imm11, u16, 11);
bounded!(/// A 20-bit `LUI` immediate.
    Imm20, u32, 20);
bounded!(/// A 4-bit `TAR` turnaround count.
    Tar4, u8, 4);
bounded!(/// A 6-bit `SET_CONFIG` payload.
    Cfg6, u8, 6);
bounded!(/// A 5-bit `RDSR` special-register selector.
    Sr5, u8, 5);
bounded!(/// A 5-bit shift amount.
    Amt5, u8, 5);
bounded!(/// A 2-bit `WAIT_ON` condition selector.
    WaitCond, u8, 2);
bounded!(/// A 9-bit `WAIT_ON` timeout.
    WaitTimeout, u16, 9);
```

- [ ] **Step 7: Run the test to verify it passes**

Run: `cargo test -p tamal-abi`
Expected: PASS (2 tests).

- [ ] **Step 8: Lint and format**

Run: `cargo clippy -p tamal-abi --all-targets -- -D warnings && cargo fmt --all --check`
Expected: no warnings, no diff.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml crates/tamal-abi/Cargo.toml crates/tamal-abi/src/lib.rs crates/tamal-abi/src/isa.rs
git commit -m "feat(abi): isa operand newtypes via bounded! macro + crate wiring"
```

---

## Task 2: `BitCount` and `ShiftOp`

The two operand types with non-trivial semantics: `BitCount` (semantic `1..=8`, stored as `n-1`) and `ShiftOp` (enum; the reserved `0b11` is unrepresentable).

**Files:**
- Modify: `crates/tamal-abi/src/isa.rs`

**Interfaces:**
- Produces: `BitCount` with `pub fn new(count: u8) -> Option<Self>` (1..=8), `pub const fn count(self) -> u8`, `pub(crate) const fn stored(self) -> u8` (0..=7), `pub(crate) const fn from_stored(s: u8) -> Self`. `ShiftOp` enum `{ Sll, Srl, Sra }` with `pub(crate) const fn bits(self) -> u8` and `pub(crate) const fn from_bits(v: u8) -> Option<Self>`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/tamal-abi/src/isa.rs`:

```rust
    #[test]
    fn bit_count_is_one_to_eight_stored_as_n_minus_one() {
        assert_eq!(BitCount::new(0), None);
        assert_eq!(BitCount::new(9), None);
        let one = BitCount::new(1).unwrap();
        let eight = BitCount::new(8).unwrap();
        assert_eq!(one.count(), 1);
        assert_eq!(one.stored(), 0);
        assert_eq!(eight.count(), 8);
        assert_eq!(eight.stored(), 7);
        assert_eq!(BitCount::from_stored(7).count(), 8);
    }

    #[test]
    fn shift_op_maps_to_two_bits_and_rejects_reserved() {
        assert_eq!(ShiftOp::Sll.bits(), 0b00);
        assert_eq!(ShiftOp::Srl.bits(), 0b01);
        assert_eq!(ShiftOp::Sra.bits(), 0b10);
        assert_eq!(ShiftOp::from_bits(0b00), Some(ShiftOp::Sll));
        assert_eq!(ShiftOp::from_bits(0b01), Some(ShiftOp::Srl));
        assert_eq!(ShiftOp::from_bits(0b10), Some(ShiftOp::Sra));
        assert_eq!(ShiftOp::from_bits(0b11), None);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tamal-abi`
Expected: FAIL — `cannot find type BitCount` / `ShiftOp`.

- [ ] **Step 3: Implement `BitCount` and `ShiftOp`**

Add below the `bounded!` invocations in `crates/tamal-abi/src/isa.rs`:

```rust
/// A `PUT_BITS`/`GET_BITS` bit count, `1..=8`. Stored as `n-1` (a 3-bit field)
/// to match the HDL `Index 8`, but constructed and read as the semantic count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitCount(u8);

impl BitCount {
    /// Construct from a count in `1..=8`, else `None`.
    ///
    /// (Not `const`: the idiomatic `(1..=8).contains(&count)` is not a `const fn`,
    /// and a hand-written two-sided comparison trips `clippy::manual_range_contains`.)
    pub fn new(count: u8) -> Option<Self> {
        if (1..=8).contains(&count) {
            Some(Self(count - 1))
        } else {
            None
        }
    }

    /// The semantic count, `1..=8`.
    pub const fn count(self) -> u8 {
        self.0 + 1
    }

    /// The stored `n-1` field value, `0..=7`.
    pub(crate) const fn stored(self) -> u8 {
        self.0
    }

    /// Construct from a stored `n-1` field (masks to 3 bits).
    pub(crate) const fn from_stored(s: u8) -> Self {
        Self(s & 0x7)
    }
}

/// A `SHIFT` operation. The reserved `0b11` encoding is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShiftOp {
    /// Logical shift left (`0b00`).
    Sll,
    /// Logical shift right (`0b01`).
    Srl,
    /// Arithmetic shift right (`0b10`).
    Sra,
}

impl ShiftOp {
    /// The 2-bit field encoding.
    pub(crate) const fn bits(self) -> u8 {
        match self {
            ShiftOp::Sll => 0b00,
            ShiftOp::Srl => 0b01,
            ShiftOp::Sra => 0b10,
        }
    }

    /// Decode a 2-bit field; the reserved `0b11` yields `None`.
    pub(crate) const fn from_bits(v: u8) -> Option<Self> {
        match v & 0x3 {
            0b00 => Some(ShiftOp::Sll),
            0b01 => Some(ShiftOp::Srl),
            0b10 => Some(ShiftOp::Sra),
            _ => None,
        }
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p tamal-abi`
Expected: PASS (4 tests).

- [ ] **Step 5: Lint, format, commit**

```bash
cargo clippy -p tamal-abi --all-targets -- -D warnings && cargo fmt --all --check
git add crates/tamal-abi/src/isa.rs
git commit -m "feat(abi): isa BitCount (1..=8) and ShiftOp operand types"
```

---

## Task 3: Field seam — `Fields`, `split_word`, `join_word`

The 32-bit word split/join, the Rust analog of the HDL `bitCoerce` at the fixed bit positions.

**Files:**
- Modify: `crates/tamal-abi/src/isa.rs`

**Interfaces:**
- Produces: `pub(crate) struct Fields { group: u8, sub: u8, rd: u8, rs1: u8, rs2: u8, imm: u16 }`; `pub(crate) fn split_word(w: u32) -> Fields`; `pub(crate) fn join_word(group: u8, sub: u8, rd: u8, rs1: u8, rs2: u8, imm: u16) -> u32`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
    use proptest::prelude::*;

    #[test]
    fn split_word_golden_bit_positions() {
        // group=0b10, sub=0xC, rd=0x1F, rs1=0x15, rs2=0x0A, imm=0x555
        let w = (0b10 << 30) | (0xC << 26) | (0x1F << 21) | (0x15 << 16) | (0x0A << 11) | 0x555;
        let f = split_word(w);
        assert_eq!(f.group, 0b10);
        assert_eq!(f.sub, 0xC);
        assert_eq!(f.rd, 0x1F);
        assert_eq!(f.rs1, 0x15);
        assert_eq!(f.rs2, 0x0A);
        assert_eq!(f.imm, 0x555);
    }

    proptest! {
        #[test]
        fn join_split_round_trip(w in any::<u32>()) {
            let f = split_word(w);
            prop_assert_eq!(join_word(f.group, f.sub, f.rd, f.rs1, f.rs2, f.imm), w);
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tamal-abi`
Expected: FAIL — `cannot find function split_word`.

- [ ] **Step 3: Implement the field seam**

Add to `crates/tamal-abi/src/isa.rs` (below `ShiftOp`, above the `#[cfg(test)]` module):

```rust
/// The six raw instruction fields, before per-opcode interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Fields {
    pub(crate) group: u8, // [31:30]
    pub(crate) sub: u8,   // [29:26]
    pub(crate) rd: u8,    // [25:21]
    pub(crate) rs1: u8,   // [20:16]
    pub(crate) rs2: u8,   // [15:11]
    pub(crate) imm: u16,  // [10:0]
}

/// Split a 32-bit word into its raw fields (the Rust analog of `bitCoerce`).
pub(crate) fn split_word(w: u32) -> Fields {
    Fields {
        group: ((w >> 30) & 0x3) as u8,
        sub: ((w >> 26) & 0xF) as u8,
        rd: ((w >> 21) & 0x1F) as u8,
        rs1: ((w >> 16) & 0x1F) as u8,
        rs2: ((w >> 11) & 0x1F) as u8,
        imm: (w & 0x7FF) as u16,
    }
}

/// Join raw fields into a 32-bit word (masking each to its width).
pub(crate) fn join_word(group: u8, sub: u8, rd: u8, rs1: u8, rs2: u8, imm: u16) -> u32 {
    ((group as u32 & 0x3) << 30)
        | ((sub as u32 & 0xF) << 26)
        | ((rd as u32 & 0x1F) << 21)
        | ((rs1 as u32 & 0x1F) << 16)
        | ((rs2 as u32 & 0x1F) << 11)
        | (imm as u32 & 0x7FF)
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p tamal-abi`
Expected: PASS (6 tests).

- [ ] **Step 5: Lint, format, commit**

```bash
cargo clippy -p tamal-abi --all-targets -- -D warnings && cargo fmt --all --check
git add crates/tamal-abi/src/isa.rs
git commit -m "feat(abi): isa Fields split_word/join_word field seam"
```

---

## Task 4: Sub-field packers

The four `imm` sub-field packers with their inverses: `PUT_BITS`, `LUI` imm20 spread, `WAIT_ON`, and `SHIFT`.

**Files:**
- Modify: `crates/tamal-abi/src/isa.rs`

**Interfaces:**
- Produces (all `pub(crate)`):
  - `fn pack_bits_imm(n_minus_1: u8, byte: u8) -> u16`; `fn unpack_bits_imm(imm: u16) -> (u8 /*n-1*/, u8 /*byte*/)`
  - `fn split_imm20(i20: u32) -> (u8 /*rs1*/, u8 /*rs2*/, u16 /*imm*/)`; `fn join_imm20(rs1: u8, rs2: u8, imm: u16) -> (u8 /*hi*/, u32 /*i20*/)`
  - `fn wait_pack(cond: u8, timeout: u16) -> u16`; `fn wait_unpack(imm: u16) -> (u8 /*cond*/, u16 /*timeout*/)`
  - `fn shift_pack(op: u8, amt: u8) -> u16`; `fn shift_unpack(imm: u16) -> (u8 /*op*/, u8 /*mid*/, u8 /*amt*/)`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
    #[test]
    fn bits_imm_layout() {
        // n-1 in imm[10:8], byte in imm[7:0]
        assert_eq!(pack_bits_imm(0x7, 0xAB), 0x7AB);
        assert_eq!(unpack_bits_imm(0x7AB), (0x7, 0xAB));
    }

    #[test]
    fn imm20_spread_reconstructs() {
        // i20 = 0x12345 -> rs1=0x01, rs2=0x04, imm=0x345
        let (rs1, rs2, imm) = split_imm20(0x12345);
        assert_eq!((rs1, rs2, imm), (0x01, 0x04, 0x345));
        let (hi, i20) = join_imm20(rs1, rs2, imm);
        assert_eq!((hi, i20), (0, 0x12345));
    }

    #[test]
    fn imm20_join_reports_reserved_bit20() {
        // rs1 bit4 set -> hi = 1 (the reserved bit)
        let (hi, _) = join_imm20(0b1_0000, 0, 0);
        assert_eq!(hi, 1);
    }

    #[test]
    fn wait_layout() {
        // cond in imm[10:9], timeout in imm[8:0]
        assert_eq!(wait_pack(0b1, 0x64), 0x264);
        assert_eq!(wait_unpack(0x264), (0b1, 0x64));
    }

    #[test]
    fn shift_layout() {
        // op in imm[10:9], reserved mid imm[8:5], amt imm[4:0]
        assert_eq!(shift_pack(0b10, 0x03), 0x403);
        assert_eq!(shift_unpack(0x403), (0b10, 0, 0x03));
        // a set mid bit is surfaced for the reserved-field check
        assert_eq!(shift_unpack(0x020), (0, 0b0001, 0));
    }

    proptest! {
        #[test]
        fn imm20_round_trip(i20 in 0u32..=0xF_FFFF) {
            let (rs1, rs2, imm) = split_imm20(i20);
            let (hi, back) = join_imm20(rs1, rs2, imm);
            prop_assert_eq!(hi, 0);
            prop_assert_eq!(back, i20);
        }

        #[test]
        fn wait_round_trip(cond in 0u8..=0x3, timeout in 0u16..=0x1FF) {
            prop_assert_eq!(wait_unpack(wait_pack(cond, timeout)), (cond, timeout));
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tamal-abi`
Expected: FAIL — `cannot find function pack_bits_imm`.

- [ ] **Step 3: Implement the packers**

Add to `crates/tamal-abi/src/isa.rs` (below `join_word`):

```rust
// --- Sub-field packers (imm interpretation per opcode). ---

/// `PUT_BITS`/`GET_BITS`: `imm[10:8] = n-1`, `imm[7:0] = byte`.
pub(crate) fn pack_bits_imm(n_minus_1: u8, byte: u8) -> u16 {
    ((n_minus_1 as u16 & 0x7) << 8) | byte as u16
}

/// Inverse of [`pack_bits_imm`].
pub(crate) fn unpack_bits_imm(imm: u16) -> (u8, u8) {
    (((imm >> 8) & 0x7) as u8, (imm & 0xFF) as u8)
}

/// `LUI`: spread a 20-bit immediate across `rs1 ++ rs2 ++ imm` (bit 20 = 0).
pub(crate) fn split_imm20(i20: u32) -> (u8, u8, u16) {
    (
        ((i20 >> 16) & 0x1F) as u8,
        ((i20 >> 11) & 0x1F) as u8,
        (i20 & 0x7FF) as u16,
    )
}

/// Inverse of [`split_imm20`]; `hi` is the reserved bit 20.
pub(crate) fn join_imm20(rs1: u8, rs2: u8, imm: u16) -> (u8, u32) {
    let temp = ((rs1 as u32 & 0x1F) << 16) | ((rs2 as u32 & 0x1F) << 11) | (imm as u32 & 0x7FF);
    (((temp >> 20) & 0x1) as u8, temp & 0xF_FFFF)
}

/// `WAIT_ON`: `imm = cond[10:9] ++ timeout[8:0]`.
pub(crate) fn wait_pack(cond: u8, timeout: u16) -> u16 {
    ((cond as u16 & 0x3) << 9) | (timeout & 0x1FF)
}

/// Inverse of [`wait_pack`].
pub(crate) fn wait_unpack(imm: u16) -> (u8, u16) {
    (((imm >> 9) & 0x3) as u8, imm & 0x1FF)
}

/// `SHIFT`: `imm = op[10:9] ++ 0[8:5] ++ amt[4:0]` (mid nibble reserved-zero).
pub(crate) fn shift_pack(op: u8, amt: u8) -> u16 {
    ((op as u16 & 0x3) << 9) | (amt as u16 & 0x1F)
}

/// Inverse of [`shift_pack`]; returns `(op, mid, amt)` so `decode` can check `mid == 0`.
pub(crate) fn shift_unpack(imm: u16) -> (u8, u8, u8) {
    (
        ((imm >> 9) & 0x3) as u8,
        ((imm >> 5) & 0xF) as u8,
        (imm & 0x1F) as u8,
    )
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p tamal-abi`
Expected: PASS (13 tests).

- [ ] **Step 5: Lint, format, commit**

```bash
cargo clippy -p tamal-abi --all-targets -- -D warnings && cargo fmt --all --check
git add crates/tamal-abi/src/isa.rs
git commit -m "feat(abi): isa imm sub-field packers (bits/imm20/wait/shift)"
```

---

## Task 5: `Instr` enum + `DecodeError`

Define the 36-variant `Instr` and the `DecodeError` taxonomy. No behavior yet — this task locks the type surface so `encode`/`decode` can be added.

**Files:**
- Modify: `crates/tamal-abi/src/isa.rs`

**Interfaces:**
- Produces: `pub enum Instr { … }` (36 variants, exactly as in the design §4.3); `pub enum DecodeError { ReservedFieldNonZero, OpcodeUnimplemented, IllegalOpcode }` deriving `thiserror::Error`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:

```rust
    #[test]
    fn instr_and_error_types_exist() {
        // Constructing representative variants proves the field types line up.
        let _ = Instr::CsAssert;
        let _ = Instr::PutByteImm(0x64);
        let _ = Instr::GetByte(Reg::new(5).unwrap());
        let _ = Instr::PutBitsImm(BitCount::new(8).unwrap(), 0xAB);
        let _ = Instr::TarImm(Tar4::new(2).unwrap());
        let _ = Instr::Beq(Reg::new(5).unwrap(), Reg::new(6).unwrap(), Imm11::new(4).unwrap());
        let _ = Instr::WaitOn(
            Reg::new(5).unwrap(),
            WaitCond::new(1).unwrap(),
            WaitTimeout::new(0x64).unwrap(),
        );
        let _ = Instr::SetConfig(Cfg6::new(0).unwrap());
        let _ = Instr::Lui(Reg::new(5).unwrap(), Imm20::new(0x12345).unwrap());
        let _ = Instr::Shift(
            Reg::new(5).unwrap(),
            Reg::new(6).unwrap(),
            ShiftOp::Sra,
            Amt5::new(3).unwrap(),
        );
        let _ = Instr::Rdsr(Reg::new(7).unwrap(), Sr5::new(0).unwrap());
        assert_ne!(DecodeError::ReservedFieldNonZero, DecodeError::IllegalOpcode);
        assert_ne!(DecodeError::OpcodeUnimplemented, DecodeError::IllegalOpcode);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tamal-abi`
Expected: FAIL — `cannot find type Instr` / `DecodeError`.

- [ ] **Step 3: Implement the `Instr` and `DecodeError` types**

Add to `crates/tamal-abi/src/isa.rs` (below the packers, above the `#[cfg(test)]` module):

```rust
/// A decoded tamal instruction — one variant per opcode across the three groups
/// (BUS `00` / CTRL `01` / DATA `10`). A 1:1 mirror of the HDL `Tamal.Isa.Instr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instr {
    // BUS group (00)
    /// Assert `CS#` (begin a frame).
    CsAssert,
    /// Deassert `CS#` (end a frame).
    CsDeassert,
    /// Shift an 8-bit immediate onto the bus, MSB-first.
    PutByteImm(u8),
    /// Shift `rs1[7:0]` onto the bus.
    PutByteReg(Reg),
    /// Sample a byte into `rd` (updates the RX CRC).
    GetByte(Reg),
    /// Shift `n` bits of an immediate byte.
    PutBitsImm(BitCount, u8),
    /// Shift `n` bits from `rs1`.
    PutBitsReg(Reg, BitCount),
    /// Sample `n` bits into `rd` (CRC-neutral).
    GetBits(Reg, BitCount),
    /// Turn the bus around for an immediate count of clocks.
    TarImm(Tar4),
    /// Turn the bus around for `rs1` clocks.
    TarReg(Reg),
    /// Drive `RESET#` asserted.
    RstAssert,
    /// Drive `RESET#` deasserted.
    RstDeassert,
    /// Sample `ALERT#` into `rd[0]`.
    GetAlert(Reg),
    // CTRL group (01)
    /// End the program, writing the status byte to the trace ring.
    Halt(u8),
    /// Branch if `rs1 == rs2`.
    Beq(Reg, Reg, Imm11),
    /// Branch if `rs1 != rs2`.
    Bne(Reg, Reg, Imm11),
    /// Branch if `rs1 < rs2` (unsigned).
    Bltu(Reg, Reg, Imm11),
    /// Branch if `rs1 >= rs2` (unsigned).
    Bgeu(Reg, Reg, Imm11),
    /// Block until a condition or timeout; `rd` records met/timed-out.
    WaitOn(Reg, WaitCond, WaitTimeout),
    /// Set the engine configuration (raw 6-bit payload; see [`crate::config`]).
    SetConfig(Cfg6),
    /// Write a MARK record (label + `rs1` payload) to the trace ring.
    Mark(Imm11, Reg),
    /// Reset the RX CRC-8 accumulator.
    CrcReset,
    // DATA group (10)
    /// `rd <- sext(imm)`.
    LoadImm(Reg, Imm11),
    /// `rd <- imm << 12`.
    Lui(Reg, Imm20),
    /// `rd <- rs1`.
    Mov(Reg, Reg),
    /// `rd <- rs1 + rs2`.
    Add(Reg, Reg, Reg),
    /// `rd <- rs1 + sext(imm)`.
    Addi(Reg, Reg, Imm11),
    /// `rd <- rs1 - rs2`.
    Sub(Reg, Reg, Reg),
    /// `rd <- rs1 & rs2`.
    And(Reg, Reg, Reg),
    /// `rd <- rs1 & sext(imm)`.
    Andi(Reg, Reg, Imm11),
    /// `rd <- rs1 | rs2`.
    Or(Reg, Reg, Reg),
    /// `rd <- rs1 | sext(imm)`.
    Ori(Reg, Reg, Imm11),
    /// `rd <- rs1 ^ rs2`.
    Xor(Reg, Reg, Reg),
    /// `rd <- rs1 ^ sext(imm)`.
    Xori(Reg, Reg, Imm11),
    /// `rd <- rs1 shifted by `amt` per [`ShiftOp`].
    Shift(Reg, Reg, ShiftOp, Amt5),
    /// Read special register `sr#` into `rd` (`sr=0` is the RX CRC-8).
    Rdsr(Reg, Sr5),
}

/// Why [`Instr::decode`] rejected a 32-bit word. Mirrors the HDL `DecodeError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// A reserved instruction field held a non-zero value.
    #[error("a reserved instruction field held a non-zero value")]
    ReservedFieldNonZero,
    /// The opcode is recognised but not implemented (kept for HDL parity; not
    /// produced by the current decoder).
    #[error("opcode recognised but not implemented")]
    OpcodeUnimplemented,
    /// The opcode is illegal: a reserved group or an unknown sub-opcode.
    #[error("illegal opcode (reserved group or unknown sub-opcode)")]
    IllegalOpcode,
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p tamal-abi`
Expected: PASS (14 tests).

- [ ] **Step 5: Lint, format, commit**

```bash
cargo clippy -p tamal-abi --all-targets -- -D warnings && cargo fmt --all --check
git add crates/tamal-abi/src/isa.rs
git commit -m "feat(abi): isa Instr enum (36 variants) + DecodeError"
```

---

## Task 6: `Instr::encode` + golden vectors + `program_to_le_bytes`

The total encode direction, pinned by an HDL-cross-checked golden table, plus the little-endian program helper.

**Files:**
- Modify: `crates/tamal-abi/src/isa.rs`

**Interfaces:**
- Consumes: all newtypes (Tasks 1–2), `join_word` + packers (Tasks 3–4), `Instr` (Task 5).
- Produces: `pub fn Instr::encode(&self) -> u32`; `pub fn program_to_le_bytes(program: &[Instr]) -> Vec<u8>`.

- [ ] **Step 1: Write the failing tests (golden table + LE helper)**

Add to the `tests` module. These `(Instr, u32)` pairs are hand-derived from the word layout and re-verified against the HDL in Task 8:

```rust
    fn golden_encode_vectors() -> Vec<(Instr, u32)> {
        vec![
            (Instr::CsAssert, 0x0000_0000),
            (Instr::CsDeassert, 0x0400_0000),
            (Instr::PutByteImm(0x64), 0x0800_0064),
            (Instr::GetByte(Reg::new(5).unwrap()), 0x10A0_0000),
            (Instr::PutBitsImm(BitCount::new(8).unwrap(), 0xAB), 0x1400_07AB),
            (Instr::TarImm(Tar4::new(2).unwrap()), 0x2000_0002),
            (Instr::Halt(0x11), 0x4000_0011),
            (
                Instr::Beq(Reg::new(5).unwrap(), Reg::new(6).unwrap(), Imm11::new(4).unwrap()),
                0x4405_3004,
            ),
            (
                Instr::WaitOn(
                    Reg::new(5).unwrap(),
                    WaitCond::new(1).unwrap(),
                    WaitTimeout::new(0x64).unwrap(),
                ),
                0x54A0_0264,
            ),
            (Instr::SetConfig(Cfg6::new(0).unwrap()), 0x5800_0000),
            (Instr::CrcReset, 0x6000_0000),
            (Instr::LoadImm(Reg::new(5).unwrap(), Imm11::new(0x0F).unwrap()), 0x80A0_000F),
            (Instr::Lui(Reg::new(5).unwrap(), Imm20::new(0x12345).unwrap()), 0x84A1_2345),
            (
                Instr::Shift(
                    Reg::new(5).unwrap(),
                    Reg::new(6).unwrap(),
                    ShiftOp::Sra,
                    Amt5::new(3).unwrap(),
                ),
                0xB0A6_0403,
            ),
            (Instr::Rdsr(Reg::new(7).unwrap(), Sr5::new(0).unwrap()), 0xB4E0_0000),
        ]
    }

    #[test]
    fn encode_matches_golden_words() {
        for (instr, word) in golden_encode_vectors() {
            assert_eq!(instr.encode(), word, "encode mismatch for {instr:?}");
        }
    }

    #[test]
    fn program_to_le_bytes_is_little_endian() {
        // 0x0800_0064 -> [0x64, 0x00, 0x00, 0x08]
        let bytes = program_to_le_bytes(&[Instr::PutByteImm(0x64)]);
        assert_eq!(bytes, vec![0x64, 0x00, 0x00, 0x08]);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tamal-abi`
Expected: FAIL — `no method named encode` / `cannot find function program_to_le_bytes`.

- [ ] **Step 3: Implement `encode` and `program_to_le_bytes`**

Add to `crates/tamal-abi/src/isa.rs` (below the `DecodeError` definition):

```rust
impl Instr {
    /// Encode this instruction to its 32-bit word. Total and infallible — the
    /// exact inverse of a successful [`Instr::decode`].
    pub fn encode(&self) -> u32 {
        use Instr::*;
        match *self {
            // BUS group (00)
            CsAssert => join_word(0b00, 0x0, 0, 0, 0, 0),
            CsDeassert => join_word(0b00, 0x1, 0, 0, 0, 0),
            PutByteImm(b) => join_word(0b00, 0x2, 0, 0, 0, b as u16),
            PutByteReg(rs) => join_word(0b00, 0x3, 0, rs.bits(), 0, 0),
            GetByte(rd) => join_word(0b00, 0x4, rd.bits(), 0, 0, 0),
            PutBitsImm(n, b) => join_word(0b00, 0x5, 0, 0, 0, pack_bits_imm(n.stored(), b)),
            PutBitsReg(rs, n) => join_word(0b00, 0x6, 0, rs.bits(), 0, pack_bits_imm(n.stored(), 0)),
            GetBits(rd, n) => join_word(0b00, 0x7, rd.bits(), 0, 0, pack_bits_imm(n.stored(), 0)),
            TarImm(n) => join_word(0b00, 0x8, 0, 0, 0, n.bits() as u16),
            TarReg(rs) => join_word(0b00, 0x9, 0, rs.bits(), 0, 0),
            RstAssert => join_word(0b00, 0xA, 0, 0, 0, 0),
            RstDeassert => join_word(0b00, 0xB, 0, 0, 0, 0),
            GetAlert(rd) => join_word(0b00, 0xC, rd.bits(), 0, 0, 0),
            // CTRL group (01)
            Halt(s) => join_word(0b01, 0x0, 0, 0, 0, s as u16),
            Beq(a, b, off) => join_word(0b01, 0x1, 0, a.bits(), b.bits(), off.bits()),
            Bne(a, b, off) => join_word(0b01, 0x2, 0, a.bits(), b.bits(), off.bits()),
            Bltu(a, b, off) => join_word(0b01, 0x3, 0, a.bits(), b.bits(), off.bits()),
            Bgeu(a, b, off) => join_word(0b01, 0x4, 0, a.bits(), b.bits(), off.bits()),
            WaitOn(rd, c, t) => join_word(0b01, 0x5, rd.bits(), 0, 0, wait_pack(c.bits(), t.bits())),
            SetConfig(p) => join_word(0b01, 0x6, 0, 0, 0, p.bits() as u16),
            Mark(lbl, rs) => join_word(0b01, 0x7, 0, rs.bits(), 0, lbl.bits()),
            CrcReset => join_word(0b01, 0x8, 0, 0, 0, 0),
            // DATA group (10)
            LoadImm(rd, i) => join_word(0b10, 0x0, rd.bits(), 0, 0, i.bits()),
            Lui(rd, i20) => {
                let (rs1, rs2, imm) = split_imm20(i20.bits());
                join_word(0b10, 0x1, rd.bits(), rs1, rs2, imm)
            }
            Mov(rd, rs) => join_word(0b10, 0x2, rd.bits(), rs.bits(), 0, 0),
            Add(rd, a, b) => join_word(0b10, 0x3, rd.bits(), a.bits(), b.bits(), 0),
            Addi(rd, a, i) => join_word(0b10, 0x4, rd.bits(), a.bits(), 0, i.bits()),
            Sub(rd, a, b) => join_word(0b10, 0x5, rd.bits(), a.bits(), b.bits(), 0),
            And(rd, a, b) => join_word(0b10, 0x6, rd.bits(), a.bits(), b.bits(), 0),
            Andi(rd, a, i) => join_word(0b10, 0x7, rd.bits(), a.bits(), 0, i.bits()),
            Or(rd, a, b) => join_word(0b10, 0x8, rd.bits(), a.bits(), b.bits(), 0),
            Ori(rd, a, i) => join_word(0b10, 0x9, rd.bits(), a.bits(), 0, i.bits()),
            Xor(rd, a, b) => join_word(0b10, 0xA, rd.bits(), a.bits(), b.bits(), 0),
            Xori(rd, a, i) => join_word(0b10, 0xB, rd.bits(), a.bits(), 0, i.bits()),
            Shift(rd, a, op, amt) => {
                join_word(0b10, 0xC, rd.bits(), a.bits(), 0, shift_pack(op.bits(), amt.bits()))
            }
            Rdsr(rd, sr) => join_word(0b10, 0xD, rd.bits(), 0, 0, sr.bits() as u16),
        }
    }
}

/// Encode a program to little-endian bytes (`0xAABBCCDD → [DD, CC, BB, AA]`),
/// ready for a `LOAD_PROGRAM` frame or a bytecode file (ISA §4 / wire §7).
pub fn program_to_le_bytes(program: &[Instr]) -> Vec<u8> {
    let mut out = Vec::with_capacity(program.len() * 4);
    for instr in program {
        out.extend_from_slice(&instr.encode().to_le_bytes());
    }
    out
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p tamal-abi`
Expected: PASS (16 tests).

- [ ] **Step 5: Lint, format, commit**

```bash
cargo clippy -p tamal-abi --all-targets -- -D warnings && cargo fmt --all --check
git add crates/tamal-abi/src/isa.rs
git commit -m "feat(abi): isa Instr::encode + program_to_le_bytes + golden vectors"
```

---

## Task 7: `Instr::decode` + reserved-field guards + per-group round-trip

The total decode direction with all reserved-field checks, and the `decode(encode(i)) == Ok(i)` round-trip law over generated instructions (all groups).

**Files:**
- Modify: `crates/tamal-abi/src/isa.rs`

**Interfaces:**
- Consumes: `split_word` + packers, `Instr`, `DecodeError`, newtype `from_bits`.
- Produces: `pub fn Instr::decode(w: u32) -> Result<Instr, DecodeError>`; private `only`, `decode_bus`, `decode_ctrl`, `decode_data`; test strategies `arb_reg`, `arb_imm11`, `arb_imm20`, `arb_tar4`, `arb_cfg6`, `arb_sr5`, `arb_amt5`, `arb_bit_count`, `arb_shift_op`, `arb_wait_cond`, `arb_wait_timeout`, `arb_bus_instr`, `arb_ctrl_instr`, `arb_data_instr`, `arb_instr`.

- [ ] **Step 1: Write the failing tests (strategies + round-trip)**

Add to the `tests` module:

```rust
    use proptest::strategy::Union;

    fn arb_reg() -> impl Strategy<Value = Reg> {
        (0u8..=31).prop_map(Reg::from_bits)
    }
    fn arb_imm11() -> impl Strategy<Value = Imm11> {
        (0u16..=0x7FF).prop_map(Imm11::from_bits)
    }
    fn arb_imm20() -> impl Strategy<Value = Imm20> {
        (0u32..=0xF_FFFF).prop_map(Imm20::from_bits)
    }
    fn arb_tar4() -> impl Strategy<Value = Tar4> {
        (0u8..=0xF).prop_map(Tar4::from_bits)
    }
    fn arb_cfg6() -> impl Strategy<Value = Cfg6> {
        (0u8..=0x3F).prop_map(Cfg6::from_bits)
    }
    fn arb_sr5() -> impl Strategy<Value = Sr5> {
        (0u8..=0x1F).prop_map(Sr5::from_bits)
    }
    fn arb_amt5() -> impl Strategy<Value = Amt5> {
        (0u8..=0x1F).prop_map(Amt5::from_bits)
    }
    fn arb_bit_count() -> impl Strategy<Value = BitCount> {
        (1u8..=8).prop_map(|n| BitCount::new(n).unwrap())
    }
    fn arb_shift_op() -> impl Strategy<Value = ShiftOp> {
        prop_oneof![Just(ShiftOp::Sll), Just(ShiftOp::Srl), Just(ShiftOp::Sra)]
    }
    fn arb_wait_cond() -> impl Strategy<Value = WaitCond> {
        (0u8..=0x3).prop_map(WaitCond::from_bits)
    }
    fn arb_wait_timeout() -> impl Strategy<Value = WaitTimeout> {
        (0u16..=0x1FF).prop_map(WaitTimeout::from_bits)
    }

    fn arb_bus_instr() -> impl Strategy<Value = Instr> {
        // `Union::new` (not `prop_oneof!`) because there are more than 10 arms.
        Union::new(vec![
            Just(Instr::CsAssert).boxed(),
            Just(Instr::CsDeassert).boxed(),
            any::<u8>().prop_map(Instr::PutByteImm).boxed(),
            arb_reg().prop_map(Instr::PutByteReg).boxed(),
            arb_reg().prop_map(Instr::GetByte).boxed(),
            (arb_bit_count(), any::<u8>()).prop_map(|(n, b)| Instr::PutBitsImm(n, b)).boxed(),
            (arb_reg(), arb_bit_count()).prop_map(|(r, n)| Instr::PutBitsReg(r, n)).boxed(),
            (arb_reg(), arb_bit_count()).prop_map(|(r, n)| Instr::GetBits(r, n)).boxed(),
            arb_tar4().prop_map(Instr::TarImm).boxed(),
            arb_reg().prop_map(Instr::TarReg).boxed(),
            Just(Instr::RstAssert).boxed(),
            Just(Instr::RstDeassert).boxed(),
            arb_reg().prop_map(Instr::GetAlert).boxed(),
        ])
    }

    fn arb_ctrl_instr() -> impl Strategy<Value = Instr> {
        Union::new(vec![
            any::<u8>().prop_map(Instr::Halt).boxed(),
            (arb_reg(), arb_reg(), arb_imm11()).prop_map(|(a, b, o)| Instr::Beq(a, b, o)).boxed(),
            (arb_reg(), arb_reg(), arb_imm11()).prop_map(|(a, b, o)| Instr::Bne(a, b, o)).boxed(),
            (arb_reg(), arb_reg(), arb_imm11()).prop_map(|(a, b, o)| Instr::Bltu(a, b, o)).boxed(),
            (arb_reg(), arb_reg(), arb_imm11()).prop_map(|(a, b, o)| Instr::Bgeu(a, b, o)).boxed(),
            (arb_reg(), arb_wait_cond(), arb_wait_timeout())
                .prop_map(|(r, c, t)| Instr::WaitOn(r, c, t))
                .boxed(),
            arb_cfg6().prop_map(Instr::SetConfig).boxed(),
            (arb_imm11(), arb_reg()).prop_map(|(l, r)| Instr::Mark(l, r)).boxed(),
            Just(Instr::CrcReset).boxed(),
        ])
    }

    fn arb_data_instr() -> impl Strategy<Value = Instr> {
        // `Union::new` (not `prop_oneof!`) because there are more than 10 arms.
        Union::new(vec![
            (arb_reg(), arb_imm11()).prop_map(|(r, i)| Instr::LoadImm(r, i)).boxed(),
            (arb_reg(), arb_imm20()).prop_map(|(r, i)| Instr::Lui(r, i)).boxed(),
            (arb_reg(), arb_reg()).prop_map(|(d, s)| Instr::Mov(d, s)).boxed(),
            (arb_reg(), arb_reg(), arb_reg()).prop_map(|(d, a, b)| Instr::Add(d, a, b)).boxed(),
            (arb_reg(), arb_reg(), arb_imm11()).prop_map(|(d, a, i)| Instr::Addi(d, a, i)).boxed(),
            (arb_reg(), arb_reg(), arb_reg()).prop_map(|(d, a, b)| Instr::Sub(d, a, b)).boxed(),
            (arb_reg(), arb_reg(), arb_reg()).prop_map(|(d, a, b)| Instr::And(d, a, b)).boxed(),
            (arb_reg(), arb_reg(), arb_imm11()).prop_map(|(d, a, i)| Instr::Andi(d, a, i)).boxed(),
            (arb_reg(), arb_reg(), arb_reg()).prop_map(|(d, a, b)| Instr::Or(d, a, b)).boxed(),
            (arb_reg(), arb_reg(), arb_imm11()).prop_map(|(d, a, i)| Instr::Ori(d, a, i)).boxed(),
            (arb_reg(), arb_reg(), arb_reg()).prop_map(|(d, a, b)| Instr::Xor(d, a, b)).boxed(),
            (arb_reg(), arb_reg(), arb_imm11()).prop_map(|(d, a, i)| Instr::Xori(d, a, i)).boxed(),
            (arb_reg(), arb_reg(), arb_shift_op(), arb_amt5())
                .prop_map(|(d, a, op, amt)| Instr::Shift(d, a, op, amt))
                .boxed(),
            (arb_reg(), arb_sr5()).prop_map(|(r, s)| Instr::Rdsr(r, s)).boxed(),
        ])
    }

    fn arb_instr() -> impl Strategy<Value = Instr> {
        prop_oneof![arb_bus_instr(), arb_ctrl_instr(), arb_data_instr()]
    }

    proptest! {
        #[test]
        fn bus_round_trip(i in arb_bus_instr()) {
            prop_assert_eq!(Instr::decode(i.encode()), Ok(i));
        }
        #[test]
        fn ctrl_round_trip(i in arb_ctrl_instr()) {
            prop_assert_eq!(Instr::decode(i.encode()), Ok(i));
        }
        #[test]
        fn data_round_trip(i in arb_data_instr()) {
            prop_assert_eq!(Instr::decode(i.encode()), Ok(i));
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tamal-abi`
Expected: FAIL — `no function decode` on `Instr`.

- [ ] **Step 3: Implement `decode` and the per-group decoders**

Add to `crates/tamal-abi/src/isa.rs` (below the `impl Instr { encode }` block; extend the existing `impl Instr` or add a new one):

```rust
impl Instr {
    /// Decode a 32-bit word: dispatch on the group, rebuild the instruction, and
    /// verify every reserved field is zero. Total — any word yields `Ok(Instr)`
    /// or `Err(DecodeError)`. Does **not** reject register fields >= 16.
    pub fn decode(w: u32) -> Result<Instr, DecodeError> {
        let f = split_word(w);
        match f.group {
            0b00 => decode_bus(f),
            0b01 => decode_ctrl(f),
            0b10 => decode_data(f),
            _ => Err(DecodeError::IllegalOpcode), // group 0b11 reserved
        }
    }
}

/// Accept `i` only when every reserved field is zero, else `ReservedFieldNonZero`.
fn only(reserved_zero: bool, i: Instr) -> Result<Instr, DecodeError> {
    if reserved_zero {
        Ok(i)
    } else {
        Err(DecodeError::ReservedFieldNonZero)
    }
}

fn decode_bus(f: Fields) -> Result<Instr, DecodeError> {
    use Instr::*;
    let Fields { sub, rd, rs1, rs2, imm, .. } = f;
    let (n3, byte) = unpack_bits_imm(imm);
    let imm_hi8 = (imm >> 8) & 0x7; // imm[10:8]
    let imm_hi4 = (imm >> 4) & 0x7F; // imm[10:4]
    match sub {
        0x0 => only(rd == 0 && rs1 == 0 && rs2 == 0 && imm == 0, CsAssert),
        0x1 => only(rd == 0 && rs1 == 0 && rs2 == 0 && imm == 0, CsDeassert),
        0x2 => only(rd == 0 && rs1 == 0 && rs2 == 0 && imm_hi8 == 0, PutByteImm(byte)),
        0x3 => only(rd == 0 && rs2 == 0 && imm == 0, PutByteReg(Reg::from_bits(rs1))),
        0x4 => only(rs1 == 0 && rs2 == 0 && imm == 0, GetByte(Reg::from_bits(rd))),
        0x5 => only(
            rd == 0 && rs1 == 0 && rs2 == 0,
            PutBitsImm(BitCount::from_stored(n3), byte),
        ),
        0x6 => only(
            rd == 0 && rs2 == 0 && byte == 0,
            PutBitsReg(Reg::from_bits(rs1), BitCount::from_stored(n3)),
        ),
        0x7 => only(
            rs1 == 0 && rs2 == 0 && byte == 0,
            GetBits(Reg::from_bits(rd), BitCount::from_stored(n3)),
        ),
        0x8 => only(
            rd == 0 && rs1 == 0 && rs2 == 0 && imm_hi4 == 0,
            TarImm(Tar4::from_bits((imm & 0xF) as u8)),
        ),
        0x9 => only(rd == 0 && rs2 == 0 && imm == 0, TarReg(Reg::from_bits(rs1))),
        0xA => only(rd == 0 && rs1 == 0 && rs2 == 0 && imm == 0, RstAssert),
        0xB => only(rd == 0 && rs1 == 0 && rs2 == 0 && imm == 0, RstDeassert),
        0xC => only(rs1 == 0 && rs2 == 0 && imm == 0, GetAlert(Reg::from_bits(rd))),
        _ => Err(DecodeError::IllegalOpcode),
    }
}

fn decode_ctrl(f: Fields) -> Result<Instr, DecodeError> {
    use Instr::*;
    let Fields { sub, rd, rs1, rs2, imm, .. } = f;
    let (cond, timeout) = wait_unpack(imm);
    let imm_hi8 = (imm >> 8) & 0x7; // imm[10:8]
    let imm_hi6 = (imm >> 6) & 0x1F; // imm[10:6]
    match sub {
        0x0 => only(
            rd == 0 && rs1 == 0 && rs2 == 0 && imm_hi8 == 0,
            Halt((imm & 0xFF) as u8),
        ),
        0x1 => only(rd == 0, Beq(Reg::from_bits(rs1), Reg::from_bits(rs2), Imm11::from_bits(imm))),
        0x2 => only(rd == 0, Bne(Reg::from_bits(rs1), Reg::from_bits(rs2), Imm11::from_bits(imm))),
        0x3 => only(rd == 0, Bltu(Reg::from_bits(rs1), Reg::from_bits(rs2), Imm11::from_bits(imm))),
        0x4 => only(rd == 0, Bgeu(Reg::from_bits(rs1), Reg::from_bits(rs2), Imm11::from_bits(imm))),
        0x5 => only(
            rs1 == 0 && rs2 == 0,
            WaitOn(Reg::from_bits(rd), WaitCond::from_bits(cond), WaitTimeout::from_bits(timeout)),
        ),
        0x6 => only(
            rd == 0 && rs1 == 0 && rs2 == 0 && imm_hi6 == 0,
            SetConfig(Cfg6::from_bits((imm & 0x3F) as u8)),
        ),
        0x7 => only(rd == 0 && rs2 == 0, Mark(Imm11::from_bits(imm), Reg::from_bits(rs1))),
        0x8 => only(rd == 0 && rs1 == 0 && rs2 == 0 && imm == 0, CrcReset),
        _ => Err(DecodeError::IllegalOpcode),
    }
}

fn decode_data(f: Fields) -> Result<Instr, DecodeError> {
    use Instr::*;
    let Fields { sub, rd, rs1, rs2, imm, .. } = f;
    let (hi, i20) = join_imm20(rs1, rs2, imm);
    let (sh_op, sh_mid, sh_amt) = shift_unpack(imm);
    let imm_hi5 = (imm >> 5) & 0x3F; // imm[10:5]
    match sub {
        0x0 => only(rs1 == 0 && rs2 == 0, LoadImm(Reg::from_bits(rd), Imm11::from_bits(imm))),
        0x1 => only(hi == 0, Lui(Reg::from_bits(rd), Imm20::from_bits(i20))),
        0x2 => only(rs2 == 0 && imm == 0, Mov(Reg::from_bits(rd), Reg::from_bits(rs1))),
        0x3 => only(imm == 0, Add(Reg::from_bits(rd), Reg::from_bits(rs1), Reg::from_bits(rs2))),
        0x4 => only(rs2 == 0, Addi(Reg::from_bits(rd), Reg::from_bits(rs1), Imm11::from_bits(imm))),
        0x5 => only(imm == 0, Sub(Reg::from_bits(rd), Reg::from_bits(rs1), Reg::from_bits(rs2))),
        0x6 => only(imm == 0, And(Reg::from_bits(rd), Reg::from_bits(rs1), Reg::from_bits(rs2))),
        0x7 => only(rs2 == 0, Andi(Reg::from_bits(rd), Reg::from_bits(rs1), Imm11::from_bits(imm))),
        0x8 => only(imm == 0, Or(Reg::from_bits(rd), Reg::from_bits(rs1), Reg::from_bits(rs2))),
        0x9 => only(rs2 == 0, Ori(Reg::from_bits(rd), Reg::from_bits(rs1), Imm11::from_bits(imm))),
        0xA => only(imm == 0, Xor(Reg::from_bits(rd), Reg::from_bits(rs1), Reg::from_bits(rs2))),
        0xB => only(rs2 == 0, Xori(Reg::from_bits(rd), Reg::from_bits(rs1), Imm11::from_bits(imm))),
        0xC => match ShiftOp::from_bits(sh_op) {
            Some(op) if rs2 == 0 && sh_mid == 0 => Ok(Shift(
                Reg::from_bits(rd),
                Reg::from_bits(rs1),
                op,
                Amt5::from_bits(sh_amt),
            )),
            _ => Err(DecodeError::ReservedFieldNonZero),
        },
        0xD => only(
            rs1 == 0 && rs2 == 0 && imm_hi5 == 0,
            Rdsr(Reg::from_bits(rd), Sr5::from_bits((imm & 0x1F) as u8)),
        ),
        _ => Err(DecodeError::IllegalOpcode),
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p tamal-abi`
Expected: PASS (19 tests — the three round-trip properties now hold).

- [ ] **Step 5: Lint, format, commit**

```bash
cargo clippy -p tamal-abi --all-targets -- -D warnings && cargo fmt --all --check
git add crates/tamal-abi/src/isa.rs
git commit -m "feat(abi): isa Instr::decode + reserved-field guards + round-trip laws"
```

---

## Task 8: Decode traps, canonical-or-traps law, and HDL parity cross-check

The negative/total-coverage tests that pin the reserved-field and illegal-opcode behavior, the universal `decode` law over all `u32`, and the manual HDL cross-check of the golden table.

**Files:**
- Modify: `crates/tamal-abi/src/isa.rs`

**Interfaces:**
- Consumes: everything in `isa.rs`.
- Produces: no new public API — tests only.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
    #[test]
    fn reserved_nonzero_field_traps() {
        // CS_ASSERT is all-reserved; setting imm bit 0 must trap.
        let cs_assert = Instr::CsAssert.encode();
        assert_eq!(Instr::decode(cs_assert + 1), Err(DecodeError::ReservedFieldNonZero));
    }

    #[test]
    fn reserved_shift_op_traps() {
        // DATA sub 0xC with op field 0b11 (imm[10:9]) is the reserved SHIFT op.
        let w = join_word(0b10, 0xC, 0, 0, 0, 0b11 << 9);
        assert_eq!(Instr::decode(w), Err(DecodeError::ReservedFieldNonZero));
    }

    #[test]
    fn reserved_group_is_illegal() {
        let w = 0b11u32 << 30;
        assert_eq!(Instr::decode(w), Err(DecodeError::IllegalOpcode));
    }

    #[test]
    fn unknown_sub_opcode_is_illegal() {
        // BUS sub 0xD is unassigned.
        let w = join_word(0b00, 0xD, 0, 0, 0, 0);
        assert_eq!(Instr::decode(w), Err(DecodeError::IllegalOpcode));
    }

    proptest! {
        #[test]
        fn any_word_decodes_canonical_or_traps(w in any::<u32>()) {
            match Instr::decode(w) {
                Ok(i) => prop_assert_eq!(i.encode(), w),
                Err(_) => {} // trapping is always acceptable
            }
        }

        #[test]
        fn combined_round_trip(i in arb_instr()) {
            prop_assert_eq!(Instr::decode(i.encode()), Ok(i));
        }
    }
```

- [ ] **Step 2: Run to verify pass (behavior already implemented in Task 7)**

Run: `cargo test -p tamal-abi`
Expected: PASS (25 tests). These assert existing behavior — if `any_word_decodes_canonical_or_traps` fails, a reserved-field guard in Task 7 is too lax; fix the offending `decode_*` arm before continuing.

- [ ] **Step 3: Cross-check the golden table against the HDL**

From `hdl/`, launch the REPL and print the reference words for the exact golden list (spec §6.3). Run: `cd hdl && cabal repl lib:tamal`, then at the prompt run `:m *Tamal.Isa` (enter the module's own scope, so `encode` + the constructors + `Clash.Prelude` are all visible under `NoImplicitPrelude`), then paste (the `Unsigned 32` conversion + `showHex` prints hex, since Clash's default `Show (BitVector n)` prints binary):

```haskell
import Numeric (showHex)
let hx i = "0x" ++ showHex (toInteger (unpack (encode i) :: Unsigned 32)) ""
mapM_ (putStrLn . hx)
  [ CsAssert
  , CsDeassert
  , PutByteImm 0x64
  , GetByte 5
  , PutBitsImm 7 0xAB          -- Index 8 stores n-1; 7 == count 8
  , TarImm 2
  , Halt 0x11
  , Beq 5 6 4
  , WaitOn 5 1 0x64
  , SetConfig 0
  , CrcReset
  , LoadImm 5 0x0F
  , Lui 5 0x12345
  , Shift 5 6 0b10 3
  , Rdsr 7 0
  ]
```

Confirm each printed hex equals the corresponding golden `u32` in `golden_encode_vectors()` (Task 6): `0x0, 0x4000000, 0x8000064, 0x10a00000, 0x140007ab, 0x20000002, 0x40000011, 0x44053004, 0x54a00264, 0x58000000, 0x60000000, 0x80a0000f, 0x84a12345, 0xb0a60403, 0xb4e00000` (leading zeros elided by `showHex`). If any differ, the Rust golden (and likely an `encode` arm) is wrong — fix `isa.rs`, not the HDL. Exit ghci with `:q`.

- [ ] **Step 4: Re-run the Rust suite**

Run: `cargo test -p tamal-abi`
Expected: PASS (25 tests).

- [ ] **Step 5: Lint, format, commit**

```bash
cargo clippy -p tamal-abi --all-targets -- -D warnings && cargo fmt --all --check
git add crates/tamal-abi/src/isa.rs
git commit -m "test(abi): isa decode traps + canonical-or-traps law + HDL parity check"
```

---

## Task 9: `config` module — `decode_config` + `pack`

The `SET_CONFIG` payload codec: a faithful mirror of `Tamal.Config` plus the host-only `pack` direction.

**Files:**
- Modify: `crates/tamal-abi/src/lib.rs`
- Create: `crates/tamal-abi/src/config.rs`

**Interfaces:**
- Consumes: `crate::isa::Cfg6` (its `bits` and `from_bits`).
- Produces: `pub enum Role { Controller, Target }`; `pub enum IoMode { X1, X2, X4 }`; `pub enum Sck { Sck20, Sck33, Sck50, Sck66 }`; `pub enum AlertSource { AlertPin, AlertIo1 }`; `pub struct Config { role, io_mode, sck, alert_source }`; `pub enum ConfigError { UnsupportedRole, UnsupportedIoMode, UnsupportedSck }`; `pub fn decode_config(payload: Cfg6) -> Result<Config, ConfigError>`; `pub fn Config::pack(&self) -> Cfg6`.

- [ ] **Step 1: Declare the module**

In `crates/tamal-abi/src/lib.rs`, immediately after the `pub mod isa;` line (and its doc comment), add:

```rust
/// The `SET_CONFIG` payload codec (`Role`/`IoMode`/`Sck`/`AlertSource`).
///
/// A Rust mirror of the HDL `Tamal.Config`, plus the host-only [`config::Config::pack`]
/// direction the gateware never needs.
pub mod config;
```

- [ ] **Step 2: Write the failing test module**

Create `crates/tamal-abi/src/config.rs`:

```rust
//! The `SET_CONFIG` payload codec — a Rust mirror of the HDL `Tamal.Config`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isa::Cfg6;

    #[test]
    fn pack_places_fields_at_pinned_bits() {
        let base = Config {
            role: Role::Controller,
            io_mode: IoMode::X1,
            sck: Sck::Sck20,
            alert_source: AlertSource::AlertPin,
        };
        assert_eq!(base.pack().bits(), 0x00);
        assert_eq!(Config { alert_source: AlertSource::AlertIo1, ..base }.pack().bits(), 0x01);
        assert_eq!(Config { sck: Sck::Sck33, ..base }.pack().bits(), 0x02);
        assert_eq!(Config { io_mode: IoMode::X2, ..base }.pack().bits(), 0x08);
        assert_eq!(Config { role: Role::Target, ..base }.pack().bits(), 0x20);
    }

    #[test]
    fn decode_config_accepts_only_v1() {
        let c = decode_config(Cfg6::new(0x00).unwrap()).unwrap();
        assert!(matches!(c.role, Role::Controller));
        assert!(matches!(c.io_mode, IoMode::X1));
        assert!(matches!(c.sck, Sck::Sck20));
        assert!(matches!(c.alert_source, AlertSource::AlertPin));
        let c1 = decode_config(Cfg6::new(0x01).unwrap()).unwrap();
        assert!(matches!(c1.alert_source, AlertSource::AlertIo1));
    }

    #[test]
    fn decode_config_rejects_non_v1_in_priority_order() {
        // role bit set -> UnsupportedRole (regardless of io/sck)
        assert_eq!(decode_config(Cfg6::new(0x20).unwrap()), Err(ConfigError::UnsupportedRole));
        // role ok, io != 0 -> UnsupportedIoMode
        assert_eq!(decode_config(Cfg6::new(0x08).unwrap()), Err(ConfigError::UnsupportedIoMode));
        // role/io ok, sck != 0 -> UnsupportedSck
        assert_eq!(decode_config(Cfg6::new(0x02).unwrap()), Err(ConfigError::UnsupportedSck));
    }

    #[test]
    fn decode_config_round_trips_v1() {
        for alert in [AlertSource::AlertPin, AlertSource::AlertIo1] {
            let c = Config {
                role: Role::Controller,
                io_mode: IoMode::X1,
                sck: Sck::Sck20,
                alert_source: alert,
            };
            assert_eq!(decode_config(c.pack()), Ok(c));
        }
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p tamal-abi`
Expected: FAIL — `cannot find type Config` / compile error in `config.rs`.

- [ ] **Step 4: Implement the `config` types and codec**

At the top of `crates/tamal-abi/src/config.rs` (above the `#[cfg(test)]` module):

```rust
use crate::isa::Cfg6;

/// Link role. v1 is controller-only; `Target` is reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Drive the bus as the eSPI controller (v1).
    Controller,
    /// Act as the eSPI target (reserved).
    Target,
}

/// I/O width. v1 is single-lane (`X1`); `X2`/`X4` land in Phase 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoMode {
    /// Single I/O (v1).
    X1,
    /// Dual I/O (reserved).
    X2,
    /// Quad I/O (reserved).
    X4,
}

/// SCK frequency selection. v1 accepts only 20 MHz (`Sck20`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sck {
    /// 20 MHz (v1).
    Sck20,
    /// 33 MHz (reserved).
    Sck33,
    /// 50 MHz (reserved).
    Sck50,
    /// 66 MHz (reserved).
    Sck66,
}

/// Where alerts are observed: the dedicated `ALERT#` pin or in-band on `IO[1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertSource {
    /// The dedicated `ALERT#` pin.
    AlertPin,
    /// In-band alerts on `IO[1]`.
    AlertIo1,
}

/// The decoded engine configuration (one field per `SET_CONFIG` sub-field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Link role.
    pub role: Role,
    /// I/O width.
    pub io_mode: IoMode,
    /// SCK frequency.
    pub sck: Sck,
    /// Alert observation source.
    pub alert_source: AlertSource,
}

/// Why a `SET_CONFIG` payload was rejected (each becomes a TRAP in the engine).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The selected role is not supported in v1.
    #[error("unsupported role (v1 is controller-only)")]
    UnsupportedRole,
    /// The selected I/O mode is not supported in v1.
    #[error("unsupported I/O mode (v1 is single-lane)")]
    UnsupportedIoMode,
    /// The selected SCK frequency is not supported in v1.
    #[error("unsupported SCK frequency (v1 is 20 MHz)")]
    UnsupportedSck,
}

impl Config {
    /// Pack into the 6-bit `SET_CONFIG` payload: `[5]=role · [4:3]=io_mode ·
    /// [2:1]=sck · [0]=alert_source`. Total (any `Config` packs).
    pub fn pack(&self) -> Cfg6 {
        let role = match self.role {
            Role::Controller => 0u8,
            Role::Target => 1,
        };
        let io = match self.io_mode {
            IoMode::X1 => 0u8,
            IoMode::X2 => 1,
            IoMode::X4 => 2,
        };
        let sck = match self.sck {
            Sck::Sck20 => 0u8,
            Sck::Sck33 => 1,
            Sck::Sck50 => 2,
            Sck::Sck66 => 3,
        };
        let alert = match self.alert_source {
            AlertSource::AlertPin => 0u8,
            AlertSource::AlertIo1 => 1,
        };
        Cfg6::from_bits((role << 5) | (io << 3) | (sck << 1) | alert)
    }
}

/// Decode a 6-bit `SET_CONFIG` payload into a [`Config`], v1-strict: only
/// `(Controller, X1, Sck20, *)` is accepted, matching the HDL `decodeConfig`.
pub fn decode_config(payload: Cfg6) -> Result<Config, ConfigError> {
    let p = payload.bits();
    let role = (p >> 5) & 0x1;
    let io = (p >> 3) & 0x3;
    let sck = (p >> 1) & 0x3;
    let alert = p & 0x1;
    match (role, io, sck) {
        (0b0, 0b00, 0b00) => Ok(Config {
            role: Role::Controller,
            io_mode: IoMode::X1,
            sck: Sck::Sck20,
            alert_source: if alert == 0 {
                AlertSource::AlertPin
            } else {
                AlertSource::AlertIo1
            },
        }),
        (0b1, _, _) => Err(ConfigError::UnsupportedRole),
        (_, io_, _) if io_ != 0b00 => Err(ConfigError::UnsupportedIoMode),
        _ => Err(ConfigError::UnsupportedSck),
    }
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p tamal-abi`
Expected: PASS (29 tests total: 25 `isa` + 4 `config`).

- [ ] **Step 6: Full-workspace verification, lint, format, commit**

Run: `cargo build --workspace && cargo test --workspace && cargo clippy -p tamal-abi --all-targets -- -D warnings && cargo fmt --all --check`
Expected: all pass, no warnings, no diff.

```bash
git add crates/tamal-abi/src/lib.rs crates/tamal-abi/src/config.rs
git commit -m "feat(abi): config SET_CONFIG codec (decode_config + Config::pack)"
```

---

## Self-Review (completed during planning)

**Spec coverage:**
- §2 `isa` items (Instr, newtypes, DecodeError, encode, decode, LE helper) → Tasks 1–8. ✓
- §2 `config` items → Task 9. ✓
- §3 D1 (1:1 mirror) → Task 5 variant list; D2 (checked newtypes, total encode) → Tasks 1/6; D3 (ShiftOp enum) → Task 2/7; D4 (raw Cfg6 in SetConfig) → Task 5/9; D5 (Reg 0..=31, no window) → Task 1 + `arb_reg`/canonical law Tasks 7–8; D6 (host-only pack) → Task 9; D7 (bounded! macro) → Task 1; D8 (in-crate tests) → all; D9 (HDL-cross-checked goldens) → Task 8 Step 3; D10 (TDD + proptest) → every task. ✓
- §4.5 reserved-field guards → Task 7 `decode_*`. ✓
- §6 test list (field seam, newtypes, packers, golden encode, decode traps, round-trip, canonical-or-traps, LE, config) → Tasks 3,1,4,6,8,7,8,6,9. ✓
- §7 deps/wiring → Task 1 (proptest/thiserror/lib.rs isa) + Task 9 (lib.rs config). ✓

**Placeholder scan:** No TBD/TODO/"handle edge cases"; every code step shows complete code. ✓

**Type consistency:** `Instr` variant field types (Task 5) match `encode` (Task 6) and `decode` (Task 7) usage; `Cfg6::from_bits`/`bits` (Task 1 macro) used by `config` (Task 9); `BitCount::stored`/`from_stored` (Task 2) used by `encode`/`decode`; `ShiftOp::bits`/`from_bits` (Task 2) used by `shift_pack`/`decode_data`. Golden words in Task 6 match the HDL cross-check list in Task 8. ✓

---

## Verification (final)

From the repo root:

```
cargo build --workspace
cargo test  -p tamal-abi          # 29 tests: proptest laws + golden vectors + config
cargo clippy -p tamal-abi --all-targets -- -D warnings
cargo fmt --all --check
```
