# Tamal — Cyclone V GX board target + multi-board build Design

Date: 2026-07-05
Status: Approved (design); implementation not started
Branch: `feat/cyclonev-board`
Scope: Add a **second synthesis target** — the Terasic **Cyclone V GX Starter Kit**
(C5G, `5CGXFC5C6F27C7`, Intel/Altera) — alongside the existing Digilent Arty A7-100T,
selectable from the Makefile:

```
make                  # BOARD=arty-a7 (default) — Vivado → tamal.bit
make BOARD=arty-a7    # explicit
make BOARD=cyclonev   # Quartus → tamal.sof
```

The board-agnostic core (`Tamal.Top.system`, `Tamal.Io.espiPads`, `Tamal.Domain.Dom100`)
is **untouched**. Two thin pin-binding shells sit above it — the current Arty top,
relocated to `Tamal.Board.ArtyA7`, and a new `Tamal.Board.CycloneV` that adds a
**50 MHz → 100 MHz PLL** (`Clash.Intel.ClockGen.alteraPllSync`) so both boards run
the design on `Dom100`. A per-toolchain Makefile split (`Makefile.vivado` /
`Makefile.quartus`, included by `BOARD`) drives the two flows; the eSPI bus is wired
onto the C5G 2×20 GPIO header and the host UART onto the board's UART pins.

Companion to the topEntity design (`docs/superpowers/specs/2026-07-03-tamal-topentity-design.md`)
and the IOBUF design (`.../2026-07-02-tamal-io-design.md`). Modelled on the working
sibling flow in `~/workspace/hdl/cyclonev-clash-examples` (sevenseg / blinky /
pushbutton), which is the same Clash→Quartus/C5G template family tamal's Vivado
Makefile was originally retargeted *from*.

---

## 1. Purpose & context

tamal v1 builds to a bitstream for the Arty A7-100T. The developer has no Arty on
hand but has a **Cyclone V GX Starter Kit**. The goal is to make the *same* gateware
build and program on the C5G, chosen by a Makefile switch, without disturbing the
Arty path or any tested logic.

Three facts make this cheap:

1. **The core is already board-agnostic.** `Tamal.Top.system` is
   `HiddenClockResetEnable dom`; `Tamal.Io.espiPads` is `HiddenClockResetEnable dom`;
   `Tamal.Domain.Dom100` is a plain 100 MHz domain. Only the ~50-line
   `topEntity` shell (`src/Tamal.hs`) is Arty-specific (100 MHz pin, XDC port names).
2. **Nothing imports the `Tamal` top module** — a repo-wide grep shows only
   `Tamal.Top` is imported (by `Test.Top`). The shell can be moved/renamed freely.
3. **Clash ships an Intel PLL primitive.** `clash-prelude` 1.10.0 (this project's pin)
   provides `Clash.Intel.ClockGen.alteraPllSync`, the exact primitive for the
   Cyclone V "Altera PLL" IP core — no `clash-cores` dependency needed.

The one genuinely new problem is **clocking**: the C5G oscillator is 50 MHz, but the
design (and the UART NCO, `SCK = Dom100/5`, etc.) are all sized for 100 MHz. Rather
than re-parameterize the design for 50 MHz, we insert a PLL so **both boards present
`Dom100` to `system`** — the design is identical downstream of the shell.

### Committed interfaces this piece consumes (all unchanged)

- `Tamal.Top.system :: HiddenClockResetEnable dom => Signal dom Bit -> Signal dom (Vec 4 Bit) -> Signal dom Bit -> (Signal dom Bit, Signal dom Lanes, Signal dom Bit, Signal dom Bit, Signal dom Bit, Signal dom Bit)` — `(txLine, lanesOut, csOut, sckOut, rstOut, led)`.
- `Tamal.Io.espiPads` — the per-lane tri-state boundary (4 scalar `BiSignal` lanes + sidebands + `alertSync`).
- `Tamal.Domain.Dom100` — `vSystem`-derived 100 MHz (Asynchronous, ActiveHigh, Defined).
- `Clash.Intel.ClockGen.alteraPllSync :: (HasAsynchronousReset domIn, ClocksSyncCxt t domIn, NumOutClocksSync t domIn <= 18) => Clock domIn -> Reset domIn -> t` — for one output clock, `t = (Clock domOut, Reset domOut)`; incorporates `resetSynchronizer` on the PLL `locked` output.

---

## 2. Scope & non-goals

**In scope**

- `Tamal.Board.ArtyA7` (module) — the current `Tamal` `topEntity`, relocated verbatim
  (logic unchanged; SPDX kept). `src/Tamal.hs` is deleted.
- `Tamal.Board.CycloneV` (module) — the new C5G shell: `DomInput50` 50 MHz pin →
  `alteraPllSync` → `Dom100`, `espiPads`, GPIO/UART/LED named ports, no reset port.
- `Tamal.Domain` — add `DomInput50` (50 MHz PLL-input domain).
- Build: main `Makefile` gains a `BOARD` dispatch + common Clash/stage/test/format
  rules and `include Makefile.$(TOOLCHAIN)`; `Makefile.vivado` (Arty, relocated
  recipes) and `Makefile.quartus` (C5G, new) hold the per-toolchain flows.
- `quartus/build.tcl` — project generation (device, sources, `QSYS_FILE`, SDC, pins).
- `constraints/c5g.sdc` (50 MHz `create_clock` + `derive_pll_clocks`) and
  `constraints/c5g_pins.tcl` (pin LOC + I/O standards).
- `build.cfg` — add `QUARTUS_SH/MAP/FIT/ASM/STA/PGM` (default: bare names on `PATH`).
- `tamal.cabal` — `exposed-modules`: `-Tamal`, `+Tamal.Board.ArtyA7`, `+Tamal.Board.CycloneV`.
- Docs: `README.md`, root `AGENTS.md` ("Target hardware", "HDL build flow"), `PLAN.md`.

**Out of scope (deferred / unchanged)**

- **Any change to `system`, `espiPads`, the engine, the loader, or any leaf** — the
  core is reused as-is; `cabal test` stays green with no test edits.
- **The Vivado flow internals** (`vivado/build.tcl`, `vivado/program.tcl`) — moved
  behind `Makefile.vivado` but not modified.
- **eSPI signal-integrity tuning on the GPIO header** — long jumpers limit usable
  clock rate; that is inherent to a bring-up harness, not this build change.
- **On-hardware validation on the C5G** — the build + program path is the deliverable;
  live eSPI bring-up on real silicon is a separate step (as it is for the Arty).
- **A C5G-specific unit test** — the shell is a thin BiSignal/PLL binding, validated by
  the `--verilog` codegen gate + hardware, exactly like the Arty shell (which has no
  unit test). The `system` cosim already covers all Dom100 behavior.
- **`.pof`/flash config, second output clock, manual reset button** — v1 programs the
  volatile `.sof` over JTAG; the PLL emits a single 100 MHz clock; no reset port.

---

## 3. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | **Two shells under `Tamal.Board.*`; move the Arty top there** (delete `src/Tamal.hs`). | Symmetry between the two targets; nothing imports the `Tamal` module, so the move is safe. The board-agnostic core keeps its `Tamal.*` names. |
| 2 | **Reuse `system`/`espiPads`/`Dom100` verbatim** — zero core edits. | They are already domain-polymorphic. Keeps every hedgehog/cosim test valid and the change low-risk: the only new synthesizable code is the ~55-line C5G shell. |
| 3 | **Cyclone V clocking = `alteraPllSync` 50 MHz → 100 MHz** (`DomInput50` → `Dom100`). | Presents `Dom100` to `system` on both boards, so the design is identical downstream. `alteraPllSync` is the Clash primitive for the Cyclone V "Altera PLL" IP (vs `altpllSync` for older ALTPLL families). |
| 4 | **PLL `areset` tied off; the design's reset comes from the PLL `locked` sync** — **no reset port**. | Honors the AGENTS.md no-reset-port rule: `areset = unsafeFromActiveHigh (pure False)`. `alteraPllSync` still asserts `rst100` until the PLL locks (internal `resetSynchronizer`), holding the design in reset on a stable clock — behaviorally identical to the Arty's power-up `init`, and strictly safer. |
| 5 | **Hand-written `constraints/c5g.sdc`** (`create_clock` 20 ns on `clk` + `derive_pll_clocks` + `derive_clock_uncertainty`); ignore Clash's auto-emitted `topEntity.sdc`. | The PLL-generated 100 MHz clock must be constrained via `derive_pll_clocks`; a single hand-authored SDC is predictable and mirrors the Arty's hand-written XDC. `build.tcl` adds only `c5g.sdc` (never globs `*.sdc`), so there is no duplicate-`create_clock` conflict with the staged Clash SDC. |
| 6 | **PLL IP via `QSYS_FILE` auto-generation during `quartus_map`** (fallback: explicit `qsys-generate`). | `alteraPllSync` emits an `altera_pll` instantiation **plus a `.qsys`** (reference 50 MHz, output 100 MHz). Adding it as `QSYS_FILE` lets Analysis & Synthesis generate the IP with no extra step. This is the one novel risk vs the reference examples (none use a PLL) — see §7. |
| 7 | **Makefile split by toolchain: `BOARD` → (`NAME`, `TOOLCHAIN`); `include Makefile.$(TOOLCHAIN)`.** | Developer's preferred structure. The two toolchains differ enough (Vivado single-launch non-project flow vs Quartus discrete `map/fit/asm/sta`) that keying the recipe file on the *toolchain* keeps each flow self-contained; the main Makefile stays board-agnostic (Clash codegen, staging, test, format, clean). |
| 8 | **Quartus flow mirrors `cyclonev-clash-examples`**: `quartus_sh -t build.tcl` (project) → `quartus_map → quartus_fit → quartus_asm → quartus_sta` → `quartus_pgm -m jtag -o "p;tamal.sof"` (embedded USB-Blaster). | Proven, developer-authored reference flow for this exact board; consistent with that ecosystem's staged-artefact model. |
| 9 | **Pinout: host UART on the board's UART pins (M9/L9); eSPI bus on `GPIO[0..7]`; status LED on `LEDG[0]`.** | Developer's choice for UART (the C5G has no on-board USB-UART bridge but the pins are broken out). The eSPI bus goes on the 2×20 GPIO header as requested; the status LED uses an on-board green LED (nicer than a header pin). |
| 10 | **Keep the `$(NU)` (nushell) filesystem/`cd` wrapper in both fragments.** | tamal deliberately uses nushell for Windows portability (documented in the Makefile). Staying uniform avoids a POSIX/nushell split within one repo. Override `NU` in `build.cfg.local` on hosts without `nu`. (Confirmable — the reference C5G examples use plain POSIX.) |

---

## 4. Module layout & source

### 4.1 `Tamal.Domain` — add the PLL-input domain

```haskell
-- | 100 MHz system domain (unchanged): Arty oscillator, or the C5G PLL output.
createDomain vSystem{vName="Dom100", vPeriod=hzToPeriod 100_000_000}

-- | 50 MHz reference domain for the C5G oscillator (CLOCK_50_B5B, pin R20),
-- the input to the Cyclone V PLL. Asynchronous + ActiveHigh (vSystem default),
-- satisfying alteraPllSync's HasAsynchronousReset constraint.
createDomain vSystem{vName="DomInput50", vPeriod=hzToPeriod 50_000_000}
```

### 4.2 `Tamal.Board.ArtyA7` — the relocated Arty shell (logic unchanged)

Byte-for-byte the current `src/Tamal.hs` `topEntity` (100 MHz `clk` pin, `noReset`,
`espiPads`, four scalar `inout` lanes `io0..io3`), only the module name changes:
`module Tamal.Board.ArtyA7 where`. SPDX header kept.

### 4.3 `Tamal.Board.CycloneV` — the new C5G shell

Identical structure to the Arty shell except the clock arrives at 50 MHz and is
multiplied to `Dom100` by the PLL, whose `locked`-derived reset drives the design:

```haskell
module Tamal.Board.CycloneV where

import Clash.Annotations.TH
import Clash.Intel.ClockGen (alteraPllSync)
import Clash.Prelude

import Tamal.Domain (Dom100, DomInput50)
import Tamal.Io (espiPads)
import Tamal.Top (system)

topEntity ::
  "clk"     ::: Clock DomInput50 ->          -- 50 MHz osc (CLOCK_50_B5B, R20)
  "uart_rx" ::: Signal Dom100 Bit ->
  "io0"     ::: BiSignalIn 'PullUp Dom100 1 ->
  "io1"     ::: BiSignalIn 'PullUp Dom100 1 ->
  "io2"     ::: BiSignalIn 'PullUp Dom100 1 ->
  "io3"     ::: BiSignalIn 'PullUp Dom100 1 ->
  "alert_n" ::: Signal Dom100 Bit ->
  ( "io0" ::: BiSignalOut 'PullUp Dom100 1
  , "io1" ::: BiSignalOut 'PullUp Dom100 1
  , "io2" ::: BiSignalOut 'PullUp Dom100 1
  , "io3" ::: BiSignalOut 'PullUp Dom100 1
  , "uart_tx" ::: Signal Dom100 Bit
  , "cs_n"    ::: Signal Dom100 Bit
  , "sck"     ::: Signal Dom100 Bit
  , "reset_n" ::: Signal Dom100 Bit
  , "led"     ::: Signal Dom100 Bit
  )
topEntity clk50 uartRx io0 io1 io2 io3 alertN =
  withClockResetEnable clk100 rst100 enableGen
    $ let (txLine, lanesO, csO, sckO, rstO, ledOut) = system uartRx ioIn alertIn
          (ioDrive, csPin, sckPin, rstPin, ioIn, alertIn) =
            espiPads lanesO csO sckO rstO alertN (io0 :> io1 :> io2 :> io3 :> Nil)
          (d0 :> d1 :> d2 :> d3 :> Nil) = ioDrive
       in (d0, d1, d2, d3, txLine, csPin, sckPin, rstPin, ledOut)
 where
  -- PLL areset tied off; (clk100, rst100) come from the Altera PLL. rst100 stays
  -- asserted until the PLL locks, then the design runs on the stable 100 MHz clock.
  (clk100, rst100) = alteraPllSync clk50 (unsafeFromActiveHigh (pure False))

makeTopEntity 'topEntity
```

Notes:
- The `(clk100 :: Clock Dom100, rst100 :: Reset Dom100)` element types are fixed by
  `withClockResetEnable clk100 rst100` + the `Dom100` ports, so no pattern type
  signature is needed; add one if inference complains.
- Four **scalar** `inout` lanes (`io0..io3`) — matching the *implemented* Arty shell:
  a `Vec 4` of BiSignals does not fuse to `inout` in Clash (documented in `src/Tamal.hs`).
- The `clk` port carries `DomInput50` (50 MHz); the pin/SDC files name it `clk`.

---

## 5. Cyclone V pin map & constraints

### 5.1 Pins (`constraints/c5g_pins.tcl`, sourced by `build.tcl`)

Pin locations and I/O standards from Terasic's `C5G_Default.qsf`. All other C5G pins
stay at Quartus's safe default (input tri-stated, weak pull-up).

| Signal | Dir | Port | C5G net | `PIN_` | I/O standard | Extra |
|--------|-----|------|---------|--------|--------------|-------|
| clock (50 MHz) | in | `clk` | `CLOCK_50_B5B` | R20 | 3.3-V LVTTL | PLL ref |
| UART RX | in | `uart_rx` | `UART_RX` | M9 | 2.5 V | on-board pin |
| UART TX | out | `uart_tx` | `UART_TX` | L9 | 2.5 V | on-board pin |
| eSPI IO0 | inout | `io0` | `GPIO[0]` | T21 | 3.3-V LVTTL | weak pull-up |
| eSPI IO1 | inout | `io1` | `GPIO[1]` | D26 | 3.3-V LVTTL | weak pull-up |
| eSPI IO2 | inout | `io2` | `GPIO[2]` | K25 | 3.3-V LVTTL | weak pull-up |
| eSPI IO3 | inout | `io3` | `GPIO[3]` | E26 | 3.3-V LVTTL | weak pull-up |
| eSPI SCK | out | `sck` | `GPIO[4]` | K26 | 3.3-V LVTTL | |
| eSPI CS# | out | `cs_n` | `GPIO[5]` | M26 | 3.3-V LVTTL | |
| eSPI RESET# | out | `reset_n` | `GPIO[6]` | M21 | 3.3-V LVTTL | |
| eSPI ALERT# | in | `alert_n` | `GPIO[7]` | P20 | 3.3-V LVTTL | weak pull-up |
| status LED | out | `led` | `LEDG[0]` | L7 | 2.5 V | on-board green |

Weak pull-ups on `io0..io3` + `alert_n` realize the `espiPads` `'PullUp` default
(eSPI idle-high), the C5G analogue of the Arty XDC's `PULLUP TRUE`. Assignment form:

```tcl
set_location_assignment PIN_R20 -to clk
set_instance_assignment -name IO_STANDARD "3.3-V LVTTL" -to clk
# ...
set_location_assignment PIN_T21 -to io0
set_instance_assignment -name IO_STANDARD "3.3-V LVTTL" -to io0
set_instance_assignment -name WEAK_PULL_UP_RESISTOR ON -to io0
```

### 5.2 Timing (`constraints/c5g.sdc`)

```tcl
create_clock -name clk50 -period 20.000 [get_ports {clk}]
derive_pll_clocks            ;# constrains the PLL-generated 100 MHz output
derive_clock_uncertainty
```

---

## 6. Build system

### 6.1 `make` interface

```
make [BOARD=arty-a7]   # default: Clash → Vivado → _build/Tamal.Board.ArtyA7/02-vivado/tamal.bit
make BOARD=cyclonev    #          Clash → Quartus → _build/Tamal.Board.CycloneV/02-quartus/tamal.sof
make program [BOARD=…] # JTAG flash (Vivado hw_manager / quartus_pgm USB-Blaster)
make test              # cabal test — board-agnostic, unchanged
make format[-check]    # fourmolu — unchanged
make clean             # rm -rf _build verilog
```

### 6.2 Main `Makefile` (board dispatch + common rules)

```makefile
BOARD ?= arty-a7
TOP    = topEntity
PROJ   = tamal

ifeq ($(BOARD),arty-a7)
  NAME      = Tamal.Board.ArtyA7
  TOOLCHAIN = vivado
else ifeq ($(BOARD),cyclonev)
  NAME      = Tamal.Board.CycloneV
  TOOLCHAIN = quartus
else
  $(error Unknown BOARD '$(BOARD)' — use BOARD=arty-a7 or BOARD=cyclonev)
endif

BUILDDIR = _build/$(NAME)
HDLDIR   = $(BUILDDIR)/01-hdl
CLASHOUT = verilog/$(NAME).$(TOP)

-include build.cfg
-include build.cfg.local

default: bitstream                 # 'bitstream' is defined in the toolchain fragment

# Clash → Verilog (common). Prereqs via $(wildcard) (no shell), all source levels.
$(CLASHOUT)/$(TOP).v: $(wildcard src/*.hs src/*/*.hs src/*/*/*.hs)
	$(CLASH) $(NAME) --verilog

# Stage the Clash output (…/*.v [+ *.qsys for cyclonev]) into 01-hdl/ (common).
$(HDLDIR)/$(TOP).v: $(CLASHOUT)/$(TOP).v
	$(NU) -c "rm -rf $(HDLDIR)"
	$(NU) -c "mkdir $(BUILDDIR)"
	$(NU) -c "cp -r $(CLASHOUT) $(HDLDIR)"

include Makefile.$(TOOLCHAIN)      # sets PART/DEVICE, constraints, bitstream/program

test: ; $(TEST)
# … format / format-check / clean as today …
.PHONY: default bitstream program test clean format format-check
.SECONDARY:
```

`default:` precedes the `include`, so it stays the default goal; its prereq
`bitstream` resolves to the fragment's rule.

### 6.3 `Makefile.vivado` (Arty — relocated, unchanged behavior)

Holds `PART = xc7a100tcsg324-1`, `XDC = $(abspath constraints/arty_a7.xdc)`,
`VDIR`/`BIT`, `BUILD_TCL`/`PGM_TCL`, and the existing single-launch recipes:
`bitstream: $(BIT)`; `$(BIT): $(HDLDIR)/$(TOP).v $(XDC)` → one
`vivado -mode batch -source vivado/build.tcl` run; `program:` → `vivado/program.tcl`.
(Lifted from the current Makefile with no logic change.)

### 6.4 `Makefile.quartus` (C5G — new, mirrors `cyclonev-clash-examples`)

```makefile
DEVICE = 5CGXFC5C6F27C7          # trailing "N" (lead-free) dropped — Quartus rejects it
QPROJ  = $(PROJ)
QDIR   = $(BUILDDIR)/02-quartus
SDC    = $(abspath constraints/c5g.sdc)
PINS   = $(abspath constraints/c5g_pins.tcl)
BUILD_TCL = $(abspath quartus/build.tcl)

QSF = $(QDIR)/$(QPROJ).qsf
MAP = $(QDIR)/$(QPROJ).map.rpt
FIT = $(QDIR)/$(QPROJ).fit.rpt
SOF = $(QDIR)/$(QPROJ).sof

bitstream: $(SOF)

program: $(SOF)
	$(NU) -c "cd $(QDIR); $(QUARTUS_PGM) -m jtag -o \"p;$(QPROJ).sof\""

$(SOF): $(FIT) ; $(NU) -c "cd $(QDIR); $(QUARTUS_ASM) $(QPROJ)"
$(FIT): $(MAP) ; $(NU) -c "cd $(QDIR); $(QUARTUS_FIT) $(QPROJ)"
$(MAP): $(QSF) ; $(NU) -c "cd $(QDIR); $(QUARTUS_MAP) $(QPROJ)"   # auto-generates QSYS_FILE IP

# Project generation: device, sources (glob *.v + *.qsys), SDC, pins.
$(QSF): $(HDLDIR)/$(TOP).v $(BUILD_TCL) $(SDC) $(PINS)
	$(NU) -c "rm -rf $(QDIR)"
	$(NU) -c "mkdir $(QDIR)"
	$(NU) -c "cd $(QDIR); $(QUARTUS_SH) -t $(BUILD_TCL) $(DEVICE) $(TOP) $(QPROJ) $(abspath $(HDLDIR)) $(SDC) $(PINS)"
```

### 6.5 `quartus/build.tcl` (project generation)

```tcl
lassign $argv device top proj hdldir sdc pins
package require ::quartus::project
project_new $proj -overwrite
set_global_assignment -name FAMILY "Cyclone V"
set_global_assignment -name DEVICE $device
set_global_assignment -name TOP_LEVEL_ENTITY $top
foreach v [glob -nocomplain [file join $hdldir *.v]]    { set_global_assignment -name VERILOG_FILE $v }
foreach q [glob -nocomplain [file join $hdldir *.qsys]] { set_global_assignment -name QSYS_FILE   $q }
set_global_assignment -name SDC_FILE $sdc
source $pins
project_close
```

### 6.6 `build.cfg` additions

```makefile
# Quartus Prime (Cyclone V / C5G). Defaults assume quartus_* are on PATH (per the
# developer's setup). If your install isn't on PATH, override in build.cfg.local, e.g.
#   QUARTUS_SH  = $(HOME)/altera_lite/25.1std/quartus/bin/quartus_sh
QUARTUS_SH  = quartus_sh
QUARTUS_MAP = quartus_map
QUARTUS_FIT = quartus_fit
QUARTUS_ASM = quartus_asm
QUARTUS_STA = quartus_sta
QUARTUS_PGM = quartus_pgm
```

Bare names (on `PATH`) keep the C5G tool config symmetric with the Vivado side
(`VIVADO = vivado`); a non-`PATH` install is a `build.cfg.local` override.

---

## 7. Key risk: the PLL `.qsys` in a discrete Quartus flow

None of the reference C5G examples use a PLL, so the `.qsys` handling is net-new and
is the piece most likely to need iteration at bring-up:

- **Primary plan:** add the Clash-emitted `.qsys` as `QSYS_FILE`; `quartus_map`
  (Analysis & Synthesis) generates the `altera_pll` IP automatically. The instantiated
  component name and the `.qsys` name are emitted from the *same* Clash `bbQsysIncName`,
  so they match by construction; `build.tcl` just globs `*.qsys`.
- **Fallback (if `quartus_map` cannot find the generated IP):** run `qsys-generate`
  explicitly in the `$(QSF)`/`$(MAP)` recipe before synthesis, e.g.
  `qsys-generate --synthesis=VERILOG --family="Cyclone V" --part=$(DEVICE) <name>.qsys`,
  and add the resulting `<name>/synthesis/<name>.qip` as a `QIP_FILE`. Documented here
  so the implementer can switch without redesign.

The `--verilog` codegen gate (§9) is run first to inspect the emitted `.qsys` +
`altera_pll` instantiation before the Quartus flow is exercised.

---

## 8. Files touched

```
new:
  hdl/src/Tamal/Board/ArtyA7.hs    -- relocated Arty topEntity (logic unchanged; +SPDX kept)
  hdl/src/Tamal/Board/CycloneV.hs  -- C5G shell: alteraPllSync 50→100 + GPIO/UART/LED (+SPDX)
  hdl/Makefile.vivado              -- Vivado recipes (lifted from current Makefile)
  hdl/Makefile.quartus             -- Quartus discrete flow (map/fit/asm/sta + quartus_pgm)
  hdl/quartus/build.tcl            -- Quartus project generation
  hdl/constraints/c5g.sdc          -- 50 MHz create_clock + derive_pll_clocks
  hdl/constraints/c5g_pins.tcl     -- C5G pin LOC + I/O standards

deleted:
  hdl/src/Tamal.hs                 -- moved to Tamal/Board/ArtyA7.hs

modified:
  hdl/src/Tamal/Domain.hs          -- + DomInput50 (50 MHz PLL input)
  hdl/Makefile                     -- BOARD dispatch + common rules + include Makefile.$(TOOLCHAIN)
  hdl/build.cfg                    -- + QUARTUS_* tool vars
  hdl/tamal.cabal                  -- exposed-modules: -Tamal +Tamal.Board.ArtyA7 +Tamal.Board.CycloneV
  hdl/README.md                    -- two boards
  hdl/PLAN.md                      -- note the second target
  AGENTS.md                        -- "Target hardware" + "HDL build flow" (two boards)
```

No engine/leaf/`system`/`espiPads` changes; no test-source changes.

---

## 9. Verification

From `hdl/`:

```
cabal test                                          # unchanged suite stays green (core untouched)
cabal run clash -- Tamal.Board.ArtyA7  --verilog    # Arty codegen gate (ports unchanged)
cabal run clash -- Tamal.Board.CycloneV --verilog   # C5G codegen gate — confirm:
                                                    #   • altera_pll instantiation + a *.qsys file
                                                    #   • clk (DomInput50) input + io0..io3 inout + named ports
make                    # BOARD=arty-a7 (default): Vivado → tamal.bit   (regression: Arty still builds)
make BOARD=cyclonev     # Quartus → tamal.sof                            (needs Quartus Prime + Cyclone V)
make format-check
```

Gates, in order: (1) `cabal test` green; (2) both `--verilog` gates elaborate, the C5G
one showing the PLL + `.qsys`; (3) `make` (Arty) still produces `tamal.bit`; (4)
`make BOARD=cyclonev` produces `tamal.sof`; (5) `make BOARD=cyclonev program` flashes
the C5G over its USB-Blaster. On hardware, the LED lifecycle (Waiting→Running→Done) is
the first sanity check, same as the Arty.

---

## 10. Implementation approach

Suggested order (details are the follow-up plan's job):

1. **Core-agnostic prep.** Add `DomInput50` to `Tamal.Domain`; move `src/Tamal.hs` →
   `src/Tamal/Board/ArtyA7.hs`; update `tamal.cabal` `exposed-modules`. Gate: `cabal test`
   + `cabal run clash -- Tamal.Board.ArtyA7 --verilog` both still pass (pure relocation).
2. **C5G shell.** Write `Tamal.Board.CycloneV` (PLL + GPIO/UART/LED ports). Gate:
   `cabal run clash -- Tamal.Board.CycloneV --verilog` elaborates; inspect the emitted
   `altera_pll` instance + `.qsys` + port list.
3. **Constraints.** `constraints/c5g.sdc` + `constraints/c5g_pins.tcl` against the
   confirmed port names.
4. **Build split.** Refactor `Makefile` → main + `Makefile.vivado` + `Makefile.quartus`;
   add `quartus/build.tcl`; extend `build.cfg`. Gate: `make` (Arty) unchanged; `make
   BOARD=cyclonev` reaches Quartus (→ `.sof` where the toolchain is available).
5. **Docs + close-out.** `README.md`, `AGENTS.md`, `PLAN.md`; `make format`; commit.

Each step keeps the Arty path green (regression guard) before touching the C5G path.

---

## 11. Prior art

- **`~/workspace/hdl/cyclonev-clash-examples`** (sevenseg / blinky / pushbutton) — the
  developer's working Clash→Quartus/C5G template: the `quartus_sh -t <proj>.tcl` project
  gen, the discrete `map/fit/asm/sta` stages, the `quartus_pgm -m jtag -o "p;<proj>.sof"`
  USB-Blaster program step, the `5CGXFC5C6F27C7` device string (trailing "N" dropped), and
  the `noReset = unsafeFromActiveHigh (pure False)` convention. This design adds the PLL
  (which those examples don't use) and folds the flow into the two-board Makefile.
- **`Clash.Intel.ClockGen`** (`clash-prelude` 1.10.0) — `alteraPllSync`/`unsafeAlteraPll`;
  the blackbox emits the `altera_pll` instantiation + `.qsys` (`Clash.Primitives.Intel.ClockGen`).
- **The Arty `topEntity`** (`src/Tamal.hs`) — the `makeTopEntity` + named-port + four-scalar-
  `inout` + `noReset` pattern the C5G shell mirrors.
- **Terasic `C5G_Default`** (`.qsf`/`.sdc`/`.cdf`) — pin locations, I/O standards, the
  single-device JTAG chain, and the `derive_pll_clocks` SDC idiom.
```
