##----------------------------------------------------------------------------##
#   Project Settings                                                           #
##----------------------------------------------------------------------------##
#
# Two-stage build, mirroring the sibling Clash/Quartus examples (e.g. sevenseg)
# but retargeted from Quartus / Cyclone V to the Vivado / Artix-7 toolchain:
#
#   Stage 1  Clash + GHC compile src/ and emit Verilog          (cabal)
#   Stage 2  a single non-project Vivado flow (vivado/build.tcl) (this Makefile)
#            synth -> opt -> place -> route -> bitstream, in ONE Vivado launch
#            program.tcl                                  -> JTAG flash (volatile)
#
# Vivado's tool-startup cost dominates a design this size, so the whole flow runs
# in a single `vivado -mode batch -source build.tcl` rather than one launch per
# stage. build.tcl still writes post_synth.dcp / post_route.dcp and the timing/
# utilisation/DRC reports into the build dir, so each stage stays inspectable.
#
# NAME is the Haskell module holding the top entity; TOP is the entity itself
# (`topEntity`, carrying a makeTopEntity annotation). Clash writes its HDL to
# verilog/$(NAME).$(TOP)/. PROJ names the bitstream.
#
#     make                       # bitstream
#     make program               # flash the Arty A7 over JTAG
#
# --- Shell note (Windows + nushell) ------------------------------------------
# GNU Make on Windows does NOT honour a custom SHELL: it direct-executes real
# .exe files found on PATH and otherwise falls back to cmd.exe. cmd doesn't
# understand the coreutils used for staging (`mkdir`/`rm`/`cp`). nushell does,
# and because `nu.exe` is on PATH it gets direct-executed — so every filesystem
# / `cd`-then-vivado step is wrapped in `$(NU) -c "..."` and written in nushell
# syntax: `mkdir` creates parents (no `-p`), `cp -r` (not `-a`), and `;`
# sequences commands (`&&` is unsupported in nushell). Source discovery uses the
# make built-in `$(wildcard ...)` instead of `$(shell find ...)` so it needs no
# shell at all. The `clash`/`cabal`/`vivado` tools are real executables and run
# bare. On a POSIX host, point $(NU) at a wrapper or adjust these recipes.

NAME = Tamal.Board.ArtyA7
TOP  = topEntity
PROJ = tamal

# Xilinx part for the Digilent Arty A7-100T.
PART = xc7a100tcsg324-1

# Build tree, keyed on the design (NAME). The Clash HDL is staged into 01-hdl/;
# the Vivado checkpoints, reports, and bitstream live in 02-vivado/.
BUILDDIR = _build/$(NAME)
HDLDIR   = $(BUILDDIR)/01-hdl
VDIR     = $(BUILDDIR)/02-vivado

# Where Clash drops its output.
CLASHOUT = verilog/$(NAME).$(TOP)

# Hand-written Vivado constraints (clock + pins). Absolute, because the Vivado
# recipe `cd $(VDIR)` first (Vivado writes its journal/log into the cwd).
XDC = $(abspath constraints/arty_a7.xdc)

# Tcl scripts, resolved to absolute paths from the Makefile dir (the recipes run
# from $(VDIR)).
BUILD_TCL = $(abspath vivado/build.tcl)
PGM_TCL   = $(abspath vivado/program.tcl)

# Final artefact.
BIT = $(VDIR)/$(PROJ).bit

# nushell, used to run the filesystem / cd steps (see the shell note above).
# Override in build.cfg.local if `nu` isn't on PATH.
NU = nu

# Tool paths live in build.cfg; copy it to build.cfg.local to override locally.
-include build.cfg
-include build.cfg.local

##----------------------------------------------------------------------------##
#   Build Rules                                                                #
##----------------------------------------------------------------------------##

default: bitstream

bitstream: $(BIT)

# Program the Arty A7 over its built-in JTAG. The .bit is volatile SRAM config
# (lost on power cycle); flashing the on-board SPI is intentionally out of scope.
program: $(BIT)
	$(NU) -c "cd $(VDIR); $(VIVADO) -mode batch -source $(PGM_TCL) -tclargs $(PROJ).bit"

# 3. The whole Vivado flow in one launch: synth -> opt -> place -> route ->
#    bitstream. Wipe any prior Vivado dir first so stale outputs can't survive.
$(BIT): $(HDLDIR)/$(TOP).v $(XDC)
	$(NU) -c "rm -rf $(VDIR)"
	$(NU) -c "mkdir $(VDIR)"
	$(NU) -c "cd $(VDIR); $(VIVADO) -mode batch -source $(BUILD_TCL) -tclargs $(PART) $(TOP) $(abspath $(HDLDIR)) $(XDC) $(PROJ)"

# 2. Stage the Clash-generated HDL into the build tree.
$(HDLDIR)/$(TOP).v: $(CLASHOUT)/$(TOP).v
	$(NU) -c "rm -rf $(HDLDIR)"
	$(NU) -c "mkdir $(BUILDDIR)"
	$(NU) -c "cp -r $(CLASHOUT) $(HDLDIR)"

# 1. Generate Verilog from Clash. Wired into make so the whole pipeline runs
#    from a single `make`. Prerequisites come from $(wildcard) (a make built-in,
#    no shell) over the two source levels (src/*.hs and src/*/*.hs).
$(CLASHOUT)/$(TOP).v: $(wildcard src/*.hs src/*/*.hs src/*/*/*.hs)
	$(CLASH) $(NAME) --verilog

##----------------------------------------------------------------------------##
#   Tests                                                                      #
##----------------------------------------------------------------------------##

# Run the tasty unit-test suite (tests/unittests.hs): pure-Haskell simulation,
# independent of the Vivado toolchain.
test:
	$(TEST)

##----------------------------------------------------------------------------##
#   Formatting (fourmolu)                                                      #
##----------------------------------------------------------------------------##

# Every Haskell source file. Shell-free discovery via $(wildcard) (a make
# built-in). The CI job globs hdl/**/*.hs (any depth), so keep enough levels
# here to cover the whole tree — otherwise `make format-check` could pass while
# CI checks a deeper file it skipped.
HS = $(wildcard \
  src/*.hs src/*/*.hs src/*/*/*.hs src/*/*/*/*.hs \
  tests/*.hs tests/*/*.hs tests/*/*/*.hs \
  bin/*.hs)

# Rewrite every file in place to the hdl/fourmolu.yaml style.
format:
	$(FOURMOLU) --mode inplace $(HS)

# Fail (non-zero exit) if anything is unformatted. Mirrors the CI fourmolu
# check, INCLUDING --check-idempotence, so a local pass matches CI exactly.
format-check:
	$(FOURMOLU) --check-idempotence --mode check $(HS)

##----------------------------------------------------------------------------##
#   Cleanup                                                                    #
##----------------------------------------------------------------------------##

clean:
	$(NU) -c "rm -rf _build verilog"

##----------------------------------------------------------------------------##
#   Special Targets                                                            #
##----------------------------------------------------------------------------##

.PHONY: default bitstream program test clean format format-check
.SECONDARY:
