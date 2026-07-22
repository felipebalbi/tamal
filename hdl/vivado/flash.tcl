# Persist the design to the Arty A7-100T's QSPI configuration flash so the FPGA
# boots it at power-up (no JTAG needed). Invoked by the Makefile from
# _build/Tamal.Board.ArtyA7/02-vivado/ as:
#
#   vivado -mode batch -source <abs>/vivado/flash.tcl \
#          -tclargs <part> <cfgmem_part> <bitfile> <mcsfile>
#
# Stage 1 (write_cfgmem) packs the .bit into an SPIx4 flash image (.mcs) — a pure
# file transform, no board. Stage 2 programs that image into the SPI flash
# indirectly, using the FPGA as an SPI bridge over JTAG, via the Vivado hardware
# manager (hw_server + cs_server). Where cs_server is blocked (e.g. enterprise
# WDAC) this aborts at connect_hw_server *after* the .mcs is written — you can
# still carry the generated .mcs to a host with a working hardware manager.

lassign $argv part cfgmem bitfile mcsfile

# --- Stage 1: bitstream -> flash image (.mcs) --------------------------------
# 16 MB = 128 Mbit flash, quad-SPI, design loaded at flash address 0.
write_cfgmem -force -format mcs -size 16 -interface SPIx4 \
    -loadbit "up 0x00000000 $bitfile" -file $mcsfile

# --- Stage 2: program the QSPI flash indirectly over JTAG --------------------
open_hw_manager
connect_hw_server
open_hw_target

set device [lindex [get_hw_devices xc7a100t_0] 0]
current_hw_device $device
refresh_hw_device -update_hw_probes false $device

# Associate the configuration memory (SPI flash) with the FPGA, then program it.
create_hw_cfgmem -hw_device $device [lindex [get_cfgmem_parts $cfgmem] 0]
set mem [get_property PROGRAM.HW_CFGMEM $device]
set_property PROGRAM.FILES              [list $mcsfile] $mem
set_property PROGRAM.ADDRESS_RANGE      {use_file}      $mem
set_property PROGRAM.BLANK_CHECK        0               $mem
set_property PROGRAM.ERASE              1               $mem
set_property PROGRAM.CFG_PROGRAM        1               $mem
set_property PROGRAM.VERIFY             1               $mem
set_property PROGRAM.CHECKSUM           0               $mem
set_property PROGRAM.UNUSED_PIN_TERMINATION {pull-none} $mem

# Load the flash-programming bridge bitstream into the FPGA, then write the flash.
create_hw_bitstream -hw_device $device [get_property PROGRAM.HW_CFGMEM_BITFILE $device]
program_hw_devices $device
program_hw_cfgmem -hw_cfgmem $mem

close_hw_target
disconnect_hw_server
close_hw_manager
