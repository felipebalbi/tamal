# Program the Arty A7-100T over JTAG via the Vivado hardware manager.
#
# Invoked by the Makefile from _build/Tamal.Board.ArtyA7/02-vivado/ as:
#   vivado -mode batch -source <abs>/vivado/program.tcl -tclargs <bitfile>
#
# This loads the bitstream into the FPGA's volatile SRAM configuration (lost on
# power cycle). Programming the on-board SPI flash is intentionally out of scope.

lassign $argv bitfile

open_hw_manager
connect_hw_server
open_hw_target

# The Arty A7-100T enumerates as a single xc7a100t device in the JTAG chain.
set device [lindex [get_hw_devices xc7a100t_0] 0]
current_hw_device $device
refresh_hw_device -update_hw_probes false $device

set_property PROGRAM.FILE $bitfile $device
program_hw_devices $device

close_hw_target
disconnect_hw_server
close_hw_manager
