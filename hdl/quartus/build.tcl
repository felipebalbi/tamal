# Tamal — Quartus project generation for the Terasic Cyclone V GX Starter Kit (C5G).
#
# Run by Makefile.quartus from _build/Tamal.Board.CycloneV/02-quartus/ as:
#   quartus_sh -t <abs>/quartus/build.tcl <device> <top> <proj> <hdldir> <sdc> <pins>
#
# It (re)creates the Quartus project there: device, the Clash-generated Verilog,
# the altera_pll QSYS_FILE emitted by Clash's alteraPllSync blackbox, the timing
# SDC, and the pin assignments. The discrete compile stages (quartus_map/fit/asm/
# sta) then run from the Makefile, mirroring the sibling cyclonev-clash-examples.

lassign $argv device top proj hdldir sdc pins

package require ::quartus::project

# -overwrite: clean slate each build; the Makefile owns staleness.
project_new $proj -overwrite

set_global_assignment -name NUM_PARALLEL_PROCESSORS ALL
set_global_assignment -name FAMILY "Cyclone V"
set_global_assignment -name DEVICE $device
set_global_assignment -name TOP_LEVEL_ENTITY $top

# Clash-generated Verilog (top + any submodules), staged into $hdldir by the Makefile.
set verilogs [glob -nocomplain [file join $hdldir *.v]]
if {[llength $verilogs] == 0} {
    project_close
    error "build.tcl: no Verilog sources found in $hdldir"
}
foreach v $verilogs {
    set_global_assignment -name VERILOG_FILE $v
}

# The Altera PLL IP emitted alongside the Verilog by alteraPllSync. Adding it as a
# QSYS_FILE lets quartus_map (Analysis & Synthesis) generate the IP automatically.
foreach q [glob -nocomplain [file join $hdldir *.qsys]] {
    set_global_assignment -name QSYS_FILE $q
}

# Hand-written timing (50 MHz create_clock + derive_pll_clocks). Only this SDC is
# added, so there is no duplicate-create_clock conflict with Clash's staged .sdc.
set_global_assignment -name SDC_FILE $sdc

# Pin locations + I/O standards.
source $pins

project_close
