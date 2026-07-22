## Tamal — Vivado constraints for the Digilent Arty A7-100T.
##
## Full v1 pinout: 100 MHz clock, USB-UART, the eSPI bus on two neighbouring
## Pmods (JA data / JB control), and a status LED. Pin numbers from the Arty
## A7-100 master XDC (Digilent / hex-five). The four IO lanes are scalar inout
## ports (io0..io3) — Clash lowers per-lane BiSignals to one inout each; a Vec
## of BiSignals does not (see src/Tamal/Board/ArtyA7.hs).

## ---- 100 MHz system clock (CLK100MHZ, bank 35, pin E3) ----------------------
set_property -dict { PACKAGE_PIN E3  IOSTANDARD LVCMOS33 } [get_ports { clk }]
create_clock -name sys_clk -period 10.000 [get_ports { clk }]

## ---- USB-UART (FTDI) — per Arty A7 master XDC ------------------------------
## A9 = uart_txd_in  : host→FPGA line, so it's the FPGA's RX (input).
## D10 = uart_rxd_out : FPGA→host line, so it's the FPGA's TX (output).
set_property -dict { PACKAGE_PIN A9  IOSTANDARD LVCMOS33 } [get_ports { uart_rx }]
set_property -dict { PACKAGE_PIN D10 IOSTANDARD LVCMOS33 } [get_ports { uart_tx }]

## ---- eSPI data lanes IO[3:0] — Pmod JA (bank 15), PULLUP (eSPI idle-high) ----
set_property -dict { PACKAGE_PIN G13 IOSTANDARD LVCMOS33 PULLUP TRUE } [get_ports { io0 }]
set_property -dict { PACKAGE_PIN B11 IOSTANDARD LVCMOS33 PULLUP TRUE } [get_ports { io1 }]
set_property -dict { PACKAGE_PIN A11 IOSTANDARD LVCMOS33 PULLUP TRUE } [get_ports { io2 }]
set_property -dict { PACKAGE_PIN D12 IOSTANDARD LVCMOS33 PULLUP TRUE } [get_ports { io3 }]

## ---- eSPI control/sideband — Pmod JB (bank 15) ------------------------------
set_property -dict { PACKAGE_PIN E15 IOSTANDARD LVCMOS33 } [get_ports { sck }]
set_property -dict { PACKAGE_PIN E16 IOSTANDARD LVCMOS33 } [get_ports { cs_n }]
set_property -dict { PACKAGE_PIN D15 IOSTANDARD LVCMOS33 } [get_ports { reset_n }]
set_property -dict { PACKAGE_PIN C15 IOSTANDARD LVCMOS33 PULLUP TRUE } [get_ports { alert_n }]

## ---- Status LED LD4 (green) — Waiting/Running/Done ---------------------------
set_property -dict { PACKAGE_PIN H5  IOSTANDARD LVCMOS33 } [get_ports { led }]

## ---- Configuration / QSPI flash boot (for `make program`) -------------------
## CFGBVS + CONFIG_VOLTAGE are required on Artix-7 (they also silence config DRC).
## The SPI settings let the FPGA boot the design from the on-board QSPI flash at
## power-up (MODE jumper on QSPI). These bake config-controller settings into the
## .bit; they do not affect JTAG configuration (`make load`), placement, or timing.
set_property CFGBVS VCCO                          [current_design]
set_property CONFIG_VOLTAGE 3.3                   [current_design]
set_property CONFIG_MODE SPIx4                    [current_design]
set_property BITSTREAM.CONFIG.SPI_BUSWIDTH 4      [current_design]
set_property BITSTREAM.CONFIG.CONFIGRATE 33       [current_design]
set_property BITSTREAM.GENERAL.COMPRESS TRUE      [current_design]
