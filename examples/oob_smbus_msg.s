# oob_smbus_msg.s — eSPI OOB channel: tunnel an SMBus message
#
# Sends a short Out-Of-Band message over PUT_OOB. The OOB channel tunnels
# SMBus/MCTP traffic (temperature reads, PMBus, PECI-over-SMBus, etc.) across
# the eSPI link. The packet is an eSPI OOB header followed by the raw SMBus
# bytes, and the whole thing is verified with the ACCEPT completion's CRC.
#
# Packet framing (all host-built; the tamal engine just shifts bytes):
#   CMD 0x06          PUT_OOB
#   0x21              cycle type = OOB Message
#   0x00              tag[7:4] | length[11:8]
#   0x04              length[7:0] = 4 payload bytes
#   then 4 SMBus bytes {dest, cmd, count, data}  (source addr + PEC omitted
#   for brevity; a real SMBus block would include them inside the payload).
#
# Only this command-phase block differs from the other per-channel examples.
#
# Real-world note: out of reset ONLY the Peripheral channel is live. Before
# this transfer works, a real bring-up must deassert RESET#, GET_STATUS, then
# SET_CONFIGURATION (eSPI CMD 0x22) to enable the OOB channel. Elided here to
# keep the per-channel packet in focus.

    .equ  PUT_OOB,        0x06    # eSPI PUT_OOB command opcode
    .equ  CYCLE_OOB_MSG,  0x21    # cycle type: OOB Message
    .equ  RSP_WAIT_STATE, 0x0F
    .equ  VERDICT_OK,     0x00
    .equ  VERDICT_CRC,    0x11

    .text
    .globl _start
_start:
    set_config CONTROLLER, X1, SCK20, ALERT_PIN
    cs_assert

    # --- command phase: host-built eSPI packet (the ONLY channel-specific part) ---
    put_byte PUT_OOB               # CMD:  PUT_OOB
    put_byte CYCLE_OOB_MSG         # cycle type = OOB Message
    put_byte 0x00                  # tag[7:4] | length[11:8]
    put_byte 0x04                  # length[7:0] = 4 payload bytes
    # -- tunneled SMBus payload --
    put_byte 0x10                  # dest SMBus addr  (0x08 << 1, write)
    put_byte 0x00                  # SMBus command code
    put_byte 0x01                  # byte count
    put_byte 0xAB                  # data byte
    put_byte 0xB1                  # TX CRC-8 over the 8 bytes above (poly 0x07)
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
