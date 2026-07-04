# Tamal — `tamal-asm` Assembler Design

Date: 2026-07-04
Status: Approved (design); implementation not started
Scope: The `tamal-asm` assembler (library + `tamal-asm` CLI) — a two-pass
assembler for the **full tamal ISA** that turns RISC-V-flavored tamal source into
tamal bytecode, plus a `disasm` path back to a textual listing. Builds on the
completed `tamal-abi` (`isa` + `config`). The runtime error-injection
`(seed, ratio)` machinery, macros, a data section, and numeric local labels are
explicitly deferred.

Companion to the ISA & HDL Engine design
(`docs/superpowers/specs/2026-07-01-tamal-isa-design.md`, esp. §6.1 branch
offsets and §6.3 the RISC-V assembly surface), the ALU/branch design
(`.../2026-07-01-tamal-alu-branch-design.md`, esp. §7 immediate extension and the
`li = LUI + ADDI` tiling / §7.3 the 11-bit reachability gap), and the
`tamal-abi::isa` design (`.../2026-07-03-tamal-abi-isa-design.md`). The
authoritative encoding is `tamal_abi::isa` (`Instr`, `encode`, `decode`,
`program_to_le_bytes`) and `tamal_abi::config` (`Config::pack`, `decode_config`);
the authoritative branch PC math is the built engine `hdl/src/Tamal/Engine.hs`.

---

## 1. Purpose & the layering

`tamal-asm` is Layer-1 authoring: it lets a human write the byte stream the
engine shifts onto the eSPI bus. The engine is eSPI-ignorant (it shifts
host-built bytes), so the assembler's job is purely to lower a readable,
RISC-V-flavored program into the exact 32-bit instruction words the FPGA decodes
— every packet byte, CRC byte, and turnaround width is authored explicitly (as in
the four `examples/*.s`).

The assembler is a thin, well-tested lowering over `tamal_abi::isa`: it never
invents encodings, it builds `Instr` values and calls `encode`. This keeps the
byte-exactness guarantee (the abi's proptest/HDL-cross-checked `encode`) intact
and makes the assembler's own tests about *surface → Instr* correctness, not bit
layout.

## 2. Scope & non-goals

**In scope (v1)**

- Mnemonics for **all 36 `Instr` variants** (BUS / CTRL / DATA).
- Core pseudo-ops: `li`, `nop`, `mv`, `j`, `beqz`, `bnez`.
- Directives: `.equ`, `.text`, `.globl`.
- Symbolic labels (`name:`) and branch/`j` targets.
- `#` **and** `;` single-line comments (`;` for Emacs asm-mode indentation).
- ABI + numeric registers for **x0–x15**, with helpful diagnostics for x16–x31.
- Built-in symbolic operands: `set_config` role/io/sck/alert keywords (via
  `Config::pack`, validated by `decode_config`); `rdsr CRC` (sr = 0).
- General `li` constant-tiling (any 32-bit value, ≤ 3 instructions).
- **ariadne** source diagnostics (in the CLI).
- CLI: `assemble` (`--emit bin|hex|listing`) + `disasm`.
- Assembles all four `examples/*.s`.

**Out of scope (deferred; each gets a clean "not supported in v1" diagnostic, not
a parse crash)**

- `.macro` / macro expansion.
- `.data` / `.word` / `.align` / `.option` (v1 has no data memory — the engine
  executes only the instruction BRAM).
- Numeric local labels `1f` / `1b`.
- `la` / `call` / `ret` (subroutine linkage; `JAL`/`JALR` are reserved in the ISA).
- Compile-time **error-injection** `(seed, ratio)` (a later phase; v1 authors
  illegal cycles by hand, e.g. `tar 3`).
- Auto-CRC directives (v1 hand-writes the TX CRC byte, as the examples do).
- Arithmetic operand expressions (an operand immediate is one number or one
  symbol).

## 3. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | **Thin lowering over `tamal_abi::isa`**: build `Instr`, call `encode`; never hand-pack bits. | Reuses the abi's byte-exact, HDL-cross-checked encoder; asm tests focus on surface→Instr. |
| D2 | **Full ISA instruction set + core pseudo-ops/directives**; defer macros/data/locals/linkage/error-injection. | One implementable, example-complete surface without over-building unused features (YAGNI). |
| D3 | **Two comment characters `#` and `;`.** | `#` is the RISC-V convention; `;` lets Emacs asm-mode's `;`-count indentation work. `;` is otherwise unused in the grammar. |
| D4 | **Registers limited to x0–x15**, but the full RISC-V ABI name set is recognized so x16–x31 (and `a6`/`s2`/`t3`…) get a *targeted* diagnostic. | Matches ISA §6.3 (16-register v1 window); a helpful error beats "unknown register". The assembler enforces the window (`abi::Reg` itself allows 0..31). |
| D5 | **Diagnostics: lib returns structured `Vec<Diagnostic>` (span + message + labels + help), dependency-free; the CLI renders with ariadne.** | Keeps `tamal-asm` reusable by non-CLI consumers; presentation lives at the edge. Fail-fast on lex/parse, collect-all in resolve/encode. |
| D6 | **`set_config` validates against v1 (`decode_config`) and rejects non-v1 combos at assemble time.** | A config the engine only TRAPs on (X2/X4/Target/SCK33…) tests our engine, not the DUT — not a useful program. Clean field-specific error instead. |
| D7 | **`mv` lowers to the native `Mov` instruction** (not `addi rd,rs,0`). | Reconciles ISA §6.3's `mv=addi` note: tamal has a native `Mov` op; one instruction and clearer intent. `nop` stays `addi x0,x0,0`. |
| D8 | **Branch offset = `target_addr − branch_addr` in words**, encoded as raw two's-complement `Imm11`. | Byte-matches the built engine (`Engine.hs`: taken → `pc = pc s + off`, where `pc s` is the branch's own address; not-taken → `pc + 1`). |
| D9 | **`Program` carries `Instr`s + per-instruction source spans**; `to_le_bytes` via the abi helper; `listing`/`disasm` share one `Instr → text` renderer. | The renderer is the inverse of the parse table, so the round-trip tests exercise both directions. |
| D10 | **`li` tiling is value-derived and sized in pass 1.** | `li`'s 1/2/3-word expansion depends only on the (address-independent) constant, so label addresses are correct before pass 2. |

## 4. Pipeline & module layout

Classic lex → parse → resolve → encode, each a focused file under
`crates/tamal-asm/src/`:

```
lib.rs           assemble(&str) -> Result<Program, Vec<Diagnostic>>; Program (+ to_le_bytes/words/listing)
lexer.rs         source -> Vec<Token> (each with a byte Span); skips '#' and ';' comments
parser.rs        tokens -> Vec<Line> AST: Label | Directive | Instr(mnemonic, [Operand]) with spans
symbol.rs        SymbolTable: .equ constants + label addresses; two-pass address assignment
encoder.rs       mnemonic + pseudo-op lowering -> Vec<Instr> (tamal_abi::isa builders); li tiling; branch offsets; range checks
mnemonics.rs     canonical mnemonic <-> Instr tables (parse spec + the Instr->text renderer), shared by encoder + disasm
diagnostics.rs   Diagnostic { severity, message, primary: Span, labels: Vec<(Span,String)>, help: Option<String> }
disasm.rs        bytes -> u32 words (LE) -> abi::decode -> textual listing
```

`Span = core::ops::Range<usize>` (byte offsets into the source), directly usable
by ariadne in the CLI.

### 4.1 Two-pass model

1. **Lex + parse** (fail-fast): tokens → `Vec<Line>`. A syntax error returns
   immediately as a single `Diagnostic`.
2. **Collect symbols**: evaluate `.equ` in source order (value = number or an
   already-defined symbol; no forward refs, no expressions); record `.globl`
   names.
3. **Pass 1 — address assignment**: walk lines; each instruction occupies a known
   word count (1 for most; `li` is 1/2/3 from its now-known constant). Assign each
   label the address of the next instruction. Accumulate the total word count.
4. **Pass 2 — encode** (collect-all): lower each instruction to `Instr`(s),
   resolving label refs to branch offsets and symbols to immediates, range- and
   window-checking every operand; push a `Diagnostic` per problem and keep going.
5. Enforce the **≤ 1024-word** instruction-BRAM limit.

If pass 2 produced any diagnostics, return `Err(diags)`; else `Ok(Program)`.

## 5. The surface (normative)

### 5.1 Lexical

- **Comments:** `#` or `;` → through end-of-line, dropped by the lexer.
- **Registers:** ABI names for x0–x15 —
  `zero ra sp gp tp t0 t1 t2 s0 fp s1 a0 a1 a2 a3 a4 a5` (`fp` aliases `s0`=x8) —
  and numeric `x0`..`x15`. The remaining RISC-V ABI names (`a6 a7 s2..s11 t3..t6`)
  and numeric `x16`..`x31` are recognized but rejected with
  "register maps to x16–x31, unavailable in v1's 16-register file".
- **Immediates:** decimal (`42`, `-5`), hex (`0x64`), binary (`0b1010`); or a
  symbol (a `.equ` name). No arithmetic expressions.
- **Labels:** `name:` defines an address; a bare `name` in a branch/`j` target
  references it.

### 5.2 Mnemonics → `Instr`

Operand *kind* (register vs immediate) disambiguates the imm/reg forms.

- **BUS:** `cs_assert` `cs_deassert` `rst_assert` `rst_deassert` `crc_reset` (no
  operands); `put_byte OP` (reg→`PutByteReg`, imm→`PutByteImm` u8);
  `get_byte rd`; `put_bits A,B` (A reg→`PutBitsReg rs,n`; A imm→`PutBitsImm n,byte`,
  n∈1..8); `get_bits rd,n` (n∈1..8); `tar OP` (reg→`TarReg`, imm→`TarImm` 0..15);
  `get_alert rd`.
- **CTRL:** `halt imm8`; `beq|bne|bltu|bgeu rs1,rs2,label`;
  `wait_on rd,cond,timeout` (cond 0..3, timeout 0..511); `set_config
  role,io,sck,alert`; `mark tag,rs` (tag = 11-bit numeric marker, **not** a code
  label); `crc_reset`.
- **DATA:** `load_imm rd,imm11`; `lui rd,imm20`; `mov rd,rs`;
  `add|sub|and|or|xor rd,rs1,rs2`; `addi|andi|ori|xori rd,rs1,imm11`;
  `sll|srl|sra rd,rs1,amt` (→`Shift`, amt 0..31); `rdsr rd,sr`.

**Symbolic operands:**

- `set_config`: keyword operands map to `config::{Role,IoMode,Sck,AlertSource}` —
  `controller`/`target`, `x1`/`x2`/`x4`, `sck20`/`sck33`/`sck50`/`sck66`,
  `alert_pin`/`alert_io1` (case-insensitive). Build `Config`, `pack()` to `Cfg6`,
  then `decode_config` it: a non-v1 combo yields a field-specific diagnostic
  (`UnsupportedIoMode` → "x2/x4 not available in v1", etc.). Only v1-valid configs
  emit `SetConfig(cfg6)`.
- `rdsr` `sr`: the keyword `CRC` → 0, or a numeric 0..31.

### 5.3 Pseudo-ops

- `nop` → `addi x0,x0,0`.
- `mv rd,rs` → `Mov rd rs` (native, D7).
- `j label` → `beq x0,x0,label`.
- `beqz rs,label` → `beq rs,x0,label`.
- `bnez rs,label` → `bne rs,x0,label`.
- `li rd,value` → tiling (§5.4).

### 5.4 `li` constant-tiling (any 32-bit value, ≤ 3 instructions)

`ADDI`/`LOAD_IMM` sign-extend an 11-bit immediate (`[-1024, 1023]`); `LUI` writes
`imm20` into bits `[31:12]`. For a target `V` (interpreted as `i32`):

1. `V ∈ [-1024, 1023]` → `load_imm rd, V` (**1** word).
2. Else pick `lo ∈ [-1024, 1023]` with `lo ≡ V (mod 4096)` when possible and
   `hi = (V − lo) >> 12` (fits 20 bits): `lui rd, hi` + `addi rd, rd, lo` (**2**
   words). Reachable iff `V mod 4096 ∈ [0,1023] ∪ [3072,4095]`.
3. Else `V mod 4096 ∈ [1024, 3071]` (the ISA §7.3 gap) → **3** words: `lui rd, hi`
   + `addi rd, rd, lo1` + `addi rd, rd, lo2`, with `lo1, lo2 ∈ [-1024, 1023]`,
   `lo1 + lo2 ≡ V (mod 4096)`, and `hi` adjusted for the combined carry. (Exact
   constant choice is pinned by the implementation plan's tiling tests.)

The word count is a pure function of `V` (a literal or resolved `.equ`), so it is
known in pass 1.

### 5.5 Branch offsets

`off = target_addr − branch_addr` in **word** units (matches `Engine.hs`: on a
taken branch `pc ← pc + off` where `pc` is the branch's own address). Encoded as
the raw two's-complement low-11 bits (`Imm11`). Range-checked to `[-1024, 1023]`;
since a program is ≤ 1024 words, any intra-program delta fits.

### 5.6 Directives

- `.equ NAME, VALUE` — constant; `VALUE` is a number or an already-defined symbol.
- `.text` — the only section in v1; accepted, effectively a no-op (all
  instructions stream into the instruction BRAM from address 0).
- `.globl NAME` — recorded/informational; the entry point is always word 0.
- Deferred directives (`.macro .data .word .align .option`), numeric locals, and
  `la`/`call`/`ret` → a "not supported in tamal-asm v1" diagnostic with a help hint.

## 6. Output & CLI

### 6.1 Library API

```rust
pub struct Diagnostic { /* severity, message, primary: Span, labels, help */ }

pub struct Program { /* instrs: Vec<Instr>, spans: Vec<Span> */ }
impl Program {
    pub fn words(&self) -> impl Iterator<Item = u32> + '_;   // instr.encode()
    pub fn to_le_bytes(&self) -> Vec<u8>;                     // tamal_abi::isa::program_to_le_bytes
    pub fn listing(&self, source: &str) -> String;           // addr | word | mnemonic ; source
}

pub fn assemble(source: &str) -> Result<Program, Vec<Diagnostic>>;
```

This replaces the placeholder `assemble(_) -> Result<Vec<u8>, AssembleError>`;
`AssembleError::NotImplemented` is removed. `disasm.rs` exposes
`disassemble(bytes: &[u8]) -> String` (LE words → `abi::decode` → listing).

### 6.2 CLI (`tamal-asm`, clap subcommands)

- `tamal-asm assemble <in.s> [-o <out>] [--emit bin|hex|listing]`
  - `bin` (default): raw little-endian words (loader-ready); default `-o` =
    `<in>.bin`.
  - `hex`: one `u32` per line, 8 hex digits (inspection / `$readmemh`); default
    `-o` = stdout.
  - `listing`: `addr  word      mnemonic            ; original source`; default
    `-o` = stdout.
- `tamal-asm disasm <in.bin> [-o <out>]`: LE bytes → words → `abi::decode` →
  listing; a trapping word renders as raw hex + `<illegal|reserved>`.
- On `Err(Vec<Diagnostic>)`: each `Diagnostic` → `ariadne::Report` over
  `Source::from(source)`, printed to stderr; exit non-zero.

## 7. Testing

- **Per-module units.** lexer (tokens + spans, both `#`/`;` comments, blank
  lines); parser (each line kind + representative syntax errors); symbol (two-pass
  addresses, `.equ`, `li` sizing feeding label addresses); encoder (every mnemonic
  → exact `Instr`/word; pseudo-op lowering; `li` tiling 1/2/3-word incl. the gap
  band; branch offset sign + range; register-window rejection; `set_config` v1
  validation).
- **Golden — the four examples.** Each `examples/*.s` assembles `Ok` and matches
  an expected word sequence (spot-checked anchors: `set_config controller,x1,sck20,
  alert_pin` → `0x5800_0000`; the `beq`/`bnez` back-edge offsets; the hand-written
  CRC `put_byte`s). This is the acceptance gate.
- **Round-trip (proptest).** For a random valid `Instr` (an `arb_instr` strategy in
  the asm tests), `parse(render(instr))` re-encodes to `instr.encode()` — proves
  the mnemonic table is a faithful bijection. And `disasm(program.to_le_bytes())`
  re-assembles to identical words.
- **Diagnostics.** Targeted bad inputs assert the message **and span**: x16–31 use
  (`add x20,x0,x0`); immediate overflow (`put_byte 0x100`); non-v1 config
  (`set_config controller,x2,sck20,alert_pin` → UnsupportedIoMode); undefined
  label; `> 1024` words; unsupported directive (`.macro`).

## 8. Dependencies & files

- **workspace `Cargo.toml`:** add `ariadne` to `[workspace.dependencies]`
  (`proptest` already present).
- **`crates/tamal-asm` (lib):** deps `tamal-abi`, `thiserror` (both present) — **no
  ariadne** (stays rendering-agnostic); dev-dep `proptest`.
- **`crates/tamal-asm-cli`:** deps `tamal-abi`, `tamal-asm`, `clap`, `color-eyre`
  (present) + `ariadne`.
- **Files:**

```
new:      crates/tamal-asm/src/lexer.rs
          crates/tamal-asm/src/parser.rs
          crates/tamal-asm/src/symbol.rs
          crates/tamal-asm/src/encoder.rs
          crates/tamal-asm/src/mnemonics.rs
          crates/tamal-asm/src/diagnostics.rs
          crates/tamal-asm/src/disasm.rs
modified: crates/tamal-asm/src/lib.rs         -- rewrite placeholder: assemble/Program, module wiring
          crates/tamal-asm/Cargo.toml         -- proptest dev-dep
          crates/tamal-asm-cli/src/main.rs    -- clap assemble/disasm + ariadne rendering
          crates/tamal-asm-cli/Cargo.toml     -- ariadne dep
          Cargo.toml                          -- ariadne in [workspace.dependencies]
reused:   examples/*.s                        -- golden test fixtures
```

## 9. Verification

```
cargo build --workspace
cargo test  -p tamal-asm            # units + golden examples + round-trip proptests
cargo test  -p tamal-asm-cli        # CLI smoke (if present)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

## 10. Out of scope / follow-ups (roadmap)

1. **This spec** — `tamal-asm` v1 (full ISA, core pseudo-ops/directives, ariadne
   diagnostics, `assemble`/`disasm`). ← implement next.
2. **`tamal-loader`** — ship the assembled bytecode to the rig over a transport
   (its own spec; consumes `Program::to_le_bytes` + the wire format).
3. **Assembler phase 2** — `.macro`, `.data`/`.word`, numeric local labels,
   `la`/`call`/`ret`, and compile-time error-injection `(seed, ratio)`.

## 11. Prior art

- **[mole](https://github.com/felipebalbi/mole)** — the sibling I2C/I3C rig; its
  host-side assembler lowers a readable program into the bus-engine bytecode, the
  same Layer-1-authoring role `tamal-asm` plays here.
- **[riscv-asm-manual](https://github.com/riscv-non-isa/riscv-asm-manual)** — the
  conventions the surface follows (comment char, ABI register names, `.equ`/`.text`
  /`.globl`, `li`/`mv`/`j`/`beqz`/`bnez` pseudo-ops).
