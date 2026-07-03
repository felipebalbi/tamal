# flash_completion.s — eSPI Flash Access channel: return a read completion
#
# Sends a 4-byte flash-read completion over PUT_FLASH_C. In the master-attached
# flash-sharing model the flash lives behind the eSPI controller: the target
# issues flash-read requests, and the controller answers them with completions
# carrying the requested data. This program is that answer — the data bytes
# {0xDE, 0xAD, 0xBE, 0xEF} are the flash contents being returned.
#
# Packet framing (all host-built; the tamal engine just shifts bytes):
#   CMD 0x08          PUT_FLASH_C
#   0x09              cycle type = Successful Completion With Data
#   0x00              tag[7:4] | length[11:8]
#   0x04              length[7:0] = 4 data bytes
#   then 4 flash data bytes
#
# Only this command-phase block differs from the other per-channel examples.
#
# Real-world note: out of reset ONLY the Peripheral channel is live. Before
# this transfer works, a real bring-up must deassert RESET#, GET_STATUS, then
# SET_CONFIGURATION (eSPI CMD 0x22) to enable the Flash Access channel; the
# completion below also assumes a matching outstanding request/tag from the
# target. Elided here to keep the per-channel packet in focus.

    .equ  PUT_FLASH_C,    0x08    # eSPI PUT_FLASH_C command opcode
    .equ  CYCLE_SCMPL_D,  0x09    # cycle type: Successful Completion With Data
    .equ  RSP_WAIT_STATE, 0x0F
    .equ  VERDICT_OK,     0x00
    .equ  VERDICT_CRC,    0x11

    .text
    .globl _start
_start:
    set_config CONTROLLER, X1, SCK20, ALERT_PIN
    cs_assert

    # --- command phase: host-built eSPI packet (the ONLY channel-specific part) ---
    put_byte PUT_FLASH_C           # CMD:  PUT_FLASH_C
    put_byte CYCLE_SCMPL_D         # cycle type = Successful Completion With Data
    put_byte 0x00                  # tag[7:4] | length[11:8]
    put_byte 0x04                  # length[7:0] = 4 data bytes
    # -- flash read data --
    put_byte 0xDE
    put_byte 0xAD
    put_byte 0xBE
    put_byte 0xEF
    put_byte 0xE8                  # TX CRC-8 over the 8 bytes above (poly 0x07)
    tar 2                          # legal turnaround

    # --- response phase: reactive WAIT_STATE poll + RX CRC residue verdict ---
poll:
    crc_reset
    get_byte t0                    # response code  (ACCEPT expected)
    li   t1, RSP_WAIT_STATE
    beq  t0, t1, poll              # WAIT_STATE -> keep polling
    get_byte t0                    # status [7:0]
    get_byte t0                    # status [15:8]
    get_byte t0                    # trailing CRC byte -> residue
    rdsr t2, CRC
    cs_deassert                    # end frame: CS# high (verdict-independent)
    bnez t2, bad_crc
    halt VERDICT_OK
bad_crc:
    halt VERDICT_CRC
