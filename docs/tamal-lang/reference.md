# tamal-lang reference

> **Status:** designed, not yet implemented. Terse lookup for the designed
> language; the authoritative source is
> [`docs/superpowers/specs/2026-07-20-tamal-lang-design.md`](../superpowers/specs/2026-07-20-tamal-lang-design.md).
> For teaching-level explanations, see the [language guide](language-guide.md).

---

## Keywords

**Declarations & structure**

```
import   as   pub   const   enum   fn   proc   let   reg   test
```

**Domain sugar (statements)**

```
config   frame   send   recv   wait_state   expect   crc   else
pass     fail    mark   crc_region
```

**Control flow**

```
if   else   while   do   repeat
```

**Config keywords** (operands to `config` / `set_config`)

```
role:   controller | target
io:     x1 | x2 | x4
sck:    sck20 | sck33 | sck50 | sck66
alert:  alert_pin | alert_io1
```

> v1 of the rig accepts only `controller, x1, sck20, {alert_pin | alert_io1}`.
> The rest are reserved; the backend rejects them (spec open question §8.5).

**Withheld from the surface** (compiler-owned; you write structured control flow
instead — see [language guide §8](language-guide.md#8-structured-control-flow)):

```
beq   bne   bltu   bgeu   j   beqz   bnez        # branch mnemonics
```

Raw non-branch instruction statements (`cs_assert`, `put_byte`, `tar`,
`get_byte`, `mark`, `rdsr`, `crc_reset`, …) *are* available and lower 1:1.

---

## Builtins

| Builtin | Signature | Meaning |
|---|---|---|
| `crc8(b)` | `bytes -> byte` | CRC-8, poly `0x07`, init `0x00`, MSB-first, no reflection, no final XOR |
| `len(b)` | `bytes -> int` | number of bytes |
| `lo(n)` | `int -> byte` | `n & 0xFF` |
| `hi(n)` | `int -> byte` | `(n >> 8) & 0xFF` |

`crc8` also appears as the **append sugar** `send pkt + crc8` (bare keyword, no
argument) — distinct from the value call `crc8(pkt)`. See
[language guide §5](language-guide.md#5-crc8--the-three-crc-surfaces).

The residue law the CRC obeys: `crc8(msg ++ [crc8(msg)]) == 0`. Pinned vector:
`crc8([0x02]) == 0x0E`.

---

## Types

| Type | Range / form | Runtime? |
|---|---|---|
| `byte` | `0..=255` | compile-time |
| `int` | 32-bit compile-time integer | compile-time |
| `bytes` | byte-string, e.g. `[0x44, 0x00]` | compile-time |
| `bool` | condition | compile-time |
| `enum` | named `byte` set | compile-time |
| `reg` | a named runtime register variable | runtime |

`crc8()` / `len()` / `lo()` / `hi()` require compile-time arguments; applying
`crc8()` to a `reg` is a type error.

---

## Literals

```
100          0x64         0b0110_0100         0xDE_AD        -1      // numbers
[0x00, 0x64]                                                        // bytes literal
a ++ b                                                              // bytes concat
```

`_` digit separators are ignored. Numbers are range-checked at the point of use.

---

## Operators

| Class | Operators |
|---|---|
| unary | `~`  `-` |
| multiplicative | `*`  `/`  `%` |
| additive | `+`  `-` |
| shift | `<<`  `>>` |
| bitwise | `&`  `^`  `\|` |
| comparison | `==`  `!=`  `<`  `<=`  `>`  `>=` |
| `bytes` | `++` (concat), indexing, `len()` |

Precedence follows the usual C/Rust convention (unary tightest, then
multiplicative, additive, shift, bitwise, comparison). The spec does not pin an
exact precedence grammar; **parenthesize when in doubt.**

---

## Comments

```
// line
/* block (no nesting) */
```

---

## The sugar → assembly lowering table

Reproduced from spec §5. This is the contract every construct honors; verify any
of it with `tamalc compile … --emit asm`.

| HLL construct | Lowers to |
|---|---|
| instruction statement (`cs_assert`, `put_byte b`, `tar 2`, `get_byte r`, `mark`, `rdsr`, …) | the identical mnemonic, 1:1 |
| `const` / `enum` / `let` used as operand | folded immediate (const-eval) |
| `send [a,b,c]` | `put_byte a` · `put_byte b` · `put_byte c` |
| `send x + crc8` / `crc_region { … }` | the emitted bytes, then `put_byte <folded crc8>` |
| `crc8(pkt)` in operand position | folded literal byte |
| `recv a, b, c` / `recv N` | one `get_byte` per name/count into allocated regs (`_` = discard); names only the post-response bytes |
| `wait_state` [`name`] | the `crc_reset` / `get_byte` / `li` / `beq` poll idiom; consumes the response-code byte, optionally binding the terminal (non-WAIT_STATE) byte to `name` |
| `do { … } while r == K` / `while` / `if`/`else` | `beq`/`bne`/`bltu`/`bgeu` + `j`, gensym'd labels |
| `repeat N { … }` | compile-time unroll ×N |
| `expect crc else X` | residue read + branch; on failure `frame` deasserts CS, then `halt X` |
| `pass` / `fail X` | `halt 0x00` / `halt X` |
| `frame { … }` | `cs_assert` · body · `cs_deassert` on every exit |
| `config …` | `set_config` with the same role/io/sck/alert keywords |
| `proc` call | hygienic inline expansion |
| `fn` call | replaced by its returned const value |
| `test NAME { … }` | `.globl _start` · `_start:` · body (must reach `halt`) |

---

## Field bounds

Every field is range-checked at compile time; out-of-range is an error, never a
wrap. (From the `tamal-asm` encoder and ISA.)

| Field | Range |
|---|---|
| `put_byte` immediate, `halt` status, any wire `byte` | `0..=255` |
| `tar` width | `0..=15` |
| `mark` tag | `0..=2047` |
| `put_bits` / `get_bits` count | `1..=8` |
| shift amount (`sll`/`srl`/`sra`) | `0..=31` |
| `rdsr` special register | `crc`, or `0..=31` |
| `wait_on` condition / timeout | `0..=3` / `0..=511` |
| branch reach | `-1024..=1023` words |
| `load_imm` (signed-21) | `-1048576..=1048575` |
| `lui` (21-bit field) | `0..=0x1F_FFFF` |
| program size | `≤ 1024` words |
| live registers per scope | `≤ 15` (`x0` is zero) |

---

## Registers (for reading `--emit-asm`)

v1 has 16 physical registers, `x0`..`x15`; `x0` is hardwired zero. You never
write these — the allocator assigns them — but `--emit-asm` shows them. ABI
names accepted by the assembler:

| x# | ABI | x# | ABI | x# | ABI | x# | ABI |
|---|---|---|---|---|---|---|---|
| x0 | `zero` | x4 | `tp` | x8 | `s0`/`fp` | x12 | `a2` |
| x1 | `ra` | x5 | `t0` | x9 | `s1` | x13 | `a3` |
| x2 | `sp` | x6 | `t1` | x10 | `a0` | x14 | `a4` |
| x3 | `gp` | x7 | `t2` | x11 | `a1` | x15 | `a5` |

`x16`..`x31` (`a6`, `a7`, `s2`.., `t3`..) do not exist in v1; the compiler never
emits them.

---

## Grammar sketch

Illustrative, not authoritative — the parser defines the real grammar. `*` is
zero-or-more, `?` is optional, `|` is alternation.

```ebnf
module      = item* ;
item        = import | const | enum | fn | proc | test ;

import      = "import" ( IDENT | STRING "as" IDENT ) ;
const       = "pub"? "const" IDENT "=" expr ;
enum        = "pub"? "enum" IDENT ":" type "{" (IDENT "=" expr ","?)* "}" ;
fn          = "pub"? "fn" IDENT "(" params? ")" "->" type "{" expr "}" ;
proc        = "pub"? "proc" IDENT "(" params? ")" block ;
test        = "test" IDENT block ;

params      = param ("," param)* ;
param       = IDENT ":" type ("=" expr)? ;

block       = "{" stmt* "}" ;
stmt        = let | reg | config | frame | send | recv | wait_state
            | expect | pass | fail | mark | crc_region
            | if | while | do_while | repeat
            | raw_instr | call ;

let         = "let" IDENT "=" expr ;
reg         = "reg" IDENT ;
config      = "config" cfg_kw "," cfg_kw "," cfg_kw "," cfg_kw ;
frame       = "frame" block ;
send        = "send" ( bytes_expr ("+" "crc8")? ) ;
recv        = "recv" recv_target ("," recv_target)* ;
recv_target = IDENT | "_" ;
wait_state  = "wait_state" IDENT? ;      // optional name binds the response byte
expect      = "expect" "crc" "else" expr ;
pass        = "pass" ;
fail        = "fail" expr ;
mark        = "mark" expr "," IDENT ;
crc_region  = "crc_region" block ;

if          = "if" expr block ("else" (block | if))? ;
while       = "while" expr block ;
do_while    = "do" block "while" expr ;
repeat      = "repeat" expr block ;

call        = qualified "(" args? ")" ;
qualified   = IDENT ("." IDENT)* ;
```

---

## CLI: `tamalc`

Mirrors the sibling `tamal-asm` CLI conventions.

```
tamalc compile <input.tam> [OPTIONS]
```

| Option | Effect |
|---|---|
| `--emit bin` | raw little-endian bytecode (default, loader-ready) |
| `--emit asm` | annotated tamal assembly — the trust bridge / teaching output |
| `--emit listing` | `addr  word  mnemonic ; source` table |
| `-o, --output <PATH>` | output path (default: `<input>.bin` for bin; stdout otherwise) |
| `-L <DIR>` | add a library search path (for `import`) |
| `-I <DIR>` | add an include search path |
| `--lint` | recompute `crc8()` over each covered payload; flag literal CRC bytes that disagree unless marked deliberate-wrong |

Diagnostics carry `.tam` source spans and render with ariadne (file, line,
caret), the same machinery as `tamal-asm`.

> **A note on `--emit-asm`.** Throughout this documentation, "`--emit-asm`" is
> prose shorthand for the flag `--emit asm`. `--emit listing` (`--emit-listing`)
> is its verbose cousin, adding addresses and encoded words.

---

## Determinism guarantees (what you can rely on)

- Identical source → **byte-identical** bytecode, on every machine, every run.
- `crc8()` is `tamal_abi::crc8`, byte-for-byte the same as the wire CRC and the
  HDL `Tamal.Crc`. No second transcription exists.
- `+ crc8` and `crc_region` fold over **exactly the emitted bytes**, in emission
  order.
- The register allocator never emits `x16`..`x31`, never aliases a live
  variable, never clobbers `x0`, and never spills (exhaustion is an error).
- Imports: canonical-path dedupe, cycle detection, duplicate-different-body is an
  error. No name leaks.
- No hidden legalization: `tar` width is always explicit; a deliberately wrong
  CRC is always written loudly (`crc8(pkt) ^ 0xFF`).
