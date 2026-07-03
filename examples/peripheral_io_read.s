# peripheral_io_read.s — eSPI Peripheral channel: short I/O read
#
# Reads one byte from I/O port 0x64 (the classic 8042 keyboard-controller
# status port) using PUT_IORD_SHORT, then verifies the completion's CRC.
#
# The tamal engine is a dumb SPI shifter with ZERO eSPI knowledge: the host
# builds every byte on the wire (CMD, header, CRC). The channel-specific part
# of this program is ONLY the run of put_byte's in the command phase — the CS
# framing, WAIT_STATE poll, and CRC residue check are identical in every one
# of these examples.
#
# Real-world note: the Peripheral channel is the one channel that is already
# enabled coming out of reset, so this transfer needs the least setup. A full
# bring-up would still deassert RESET# (rst_deassert) and GET_STATUS first;
# elided here to keep the I/O read in focus.

    .equ  PUT_IORD1,      0x44    # PUT_IORD_SHORT, length = 1 byte
    .equ  RSP_WAIT_STATE, 0x0F    # eSPI WAIT_STATE response code
    .equ  VERDICT_OK,     0x00    # host verdict codes (written by halt)
    .equ  VERDICT_CRC,    0x11

    .text
    .globl _start
_start:
    set_config CONTROLLER, X1, SCK20, ALERT_PIN   # controller, x1 IO, 20 MHz
    cs_assert                       # begin frame: CS# low

    # --- command phase: host-built eSPI packet (the ONLY channel-specific part) ---
    put_byte PUT_IORD1              # CMD:  PUT_IORD_SHORT (1 byte)
    put_byte 0x00                   # addr [15:8]
    put_byte 0x64                   # addr [7:0]  -> I/O port 0x64
    put_byte 0x16                   # TX CRC-8 over the 3 bytes above (poly 0x07)
    tar 2                           # legal turnaround; tar 3 / tar 1 = deliberate TAR violation

    # --- response phase: reactive WAIT_STATE poll + RX CRC residue verdict ---
poll:
    crc_reset                       # drop any prior WAIT_STATE byte from the residue
    get_byte t0                     # response code (auto-updates RX CRC-8)
    li   t1, RSP_WAIT_STATE
    beq  t0, t1, poll               # WAIT_STATE -> keep polling
    get_byte t0                     # read data byte  (port 0x64 value)
    get_byte t0                     # status [7:0]
    get_byte t0                     # status [15:8]
    get_byte t0                     # trailing CRC byte -> drives residue to 0
    rdsr t2, CRC                    # RX CRC-8 residue (0 == good packet)
    cs_deassert                     # end frame: CS# high (verdict-independent)
    bnez t2, bad_crc
    halt VERDICT_OK
bad_crc:
    halt VERDICT_CRC
