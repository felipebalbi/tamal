# virtual_wire_pltrst.s — eSPI Virtual Wire channel: drive a virtual wire
#
# Sends a single Virtual Wire group over PUT_VWIRE that deasserts PLTRST#
# (platform reset), then verifies the ACCEPT completion's CRC.
#
# eSPI tunnels sideband signals ("virtual wires") as index/data pairs instead
# of physical pins. Index 0x03 carries {SUS_STAT#, PLTRST#, OOB_RST_WARN}; the
# data byte is {valid[3:0] << 4 | level[3:0]}. 0x22 = valid bit1 + level bit1,
# i.e. "PLTRST# is now 1 (deasserted)".
#
# As always the tamal engine knows none of this — it just shifts the four
# host-built command bytes below. Only that block differs from the other
# per-channel examples.
#
# Real-world note: out of reset ONLY the Peripheral channel is live. Before
# this transfer works, a real bring-up must deassert RESET#, GET_STATUS, then
# SET_CONFIGURATION (eSPI CMD 0x22) to enable the Virtual Wire channel. Elided
# here to keep the per-channel packet in focus.

    .equ  PUT_VWIRE,      0x04    # eSPI PUT_VWIRE command opcode
    .equ  VW_INDEX_SYS,   0x03    # VW index: SUS_STAT# / PLTRST# / OOB_RST_WARN
    .equ  VW_PLTRST_HIGH, 0x22    # valid=PLTRST#, level=1 (deasserted)
    .equ  RSP_WAIT_STATE, 0x0F
    .equ  VERDICT_OK,     0x00
    .equ  VERDICT_CRC,    0x11

    .text
    .globl _start
_start:
    set_config CONTROLLER, X1, SCK20, ALERT_PIN
    cs_assert

    # --- command phase: host-built eSPI packet (the ONLY channel-specific part) ---
    put_byte PUT_VWIRE              # CMD:  PUT_VWIRE
    put_byte 0x00                   # count = 0  -> one VW group follows
    put_byte VW_INDEX_SYS           # VW index 0x03
    put_byte VW_PLTRST_HIGH         # VW data: deassert PLTRST#
    put_byte 0x89                   # TX CRC-8 over the 4 bytes above (poly 0x07)
    tar 2                           # legal turnaround

    # --- response phase: reactive WAIT_STATE poll + RX CRC residue verdict ---
poll:
    crc_reset
    get_byte t0                     # response code  (ACCEPT expected)
    li   t1, RSP_WAIT_STATE
    beq  t0, t1, poll               # WAIT_STATE -> keep polling
    get_byte t0                     # status [7:0]
    get_byte t0                     # status [15:8]
    get_byte t0                     # trailing CRC byte -> residue
    rdsr t2, CRC
    cs_deassert                     # end frame: CS# high (verdict-independent)
    bnez t2, bad_crc
    halt VERDICT_OK
bad_crc:
    halt VERDICT_CRC
