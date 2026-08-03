//! End-to-end: one `proc` — the body Plan 5 will move into the bundled `espi`
//! stdlib — drives two different eSPI channels, and each lowers to bytecode
//! byte-identical *modulo register allocation* to its hand-written `.s`.

mod common;

use common::canon;

/// The shared framing/poll/verify skeleton, written once. Plan 4a has no
/// `import` yet (that is Plan 5), so it lives in the test source.
///
/// Plan 5 moves this **body** into `espi.tam` verbatim — design §4.3 spells the
/// same seven statements. Only the signature changes there: it gains `pub` and
/// writes the default as `Verdict.Crc`, both out of Plan 4a's scope, and both
/// spellings of the same 0x11. The **call sites** genuinely do not change
/// beyond gaining the `espi.` qualifier that `import` introduces.
const COMMAND_PROC: &str = "\
proc command(pkt: bytes, ndata: int, err: byte = 0x11) {
    frame {
        send pkt + crc8
        tar 2
        wait_state
        repeat ndata { recv _ }
        recv status0, status1
        expect crc else err
    }
}
";

/// OOB channel: a tunnelled SMBus message. A write completion returns status
/// only, so `ndata = 0`; the arguments are named, as in the design's §4.3.
///
/// **Not pinned here:** §4.3 writes the named arguments in declaration order,
/// so this test cannot tell name-matching from position-matching — binding
/// `pkt` and `ndata` by index gives the identical bytecode. That distinction is
/// pinned by `consteval::tests::{binds_named_arguments_by_name,
/// rejects_a_named_argument_with_no_matching_parameter,
/// rejects_a_parameter_bound_twice}`. Reordering them here would close the gap
/// but drift from the §4.3 shape this file exists to demonstrate.
fn oob_tam() -> String {
    format!(
        "const PUT_OOB = 0x06\n\n{COMMAND_PROC}\n\
test oob_msg {{
    config controller, x1, sck20, alert_pin
    command(
        pkt = [PUT_OOB, 0x21, 0x00, 0x04,
               0x10, 0x00, 0x01, 0xAB],
        ndata = 0,
    )
    pass
}}
"
    )
}

/// Peripheral channel: a short I/O read. The header is built by a `fn`, and the
/// completion carries one payload byte, so `ndata = 1` — passed positionally.
fn peripheral_tam() -> String {
    format!(
        "const PUT_IORD1 = 0x44\n\
fn iord_hdr(op: byte, addr: int) -> bytes {{ [op, hi(addr), lo(addr)] }}\n\n{COMMAND_PROC}\n\
test io_read {{
    config controller, x1, sck20, alert_pin
    command(iord_hdr(PUT_IORD1, 0x0064), 1)
    pass
}}
"
    )
}

#[test]
fn oob_smbus_msg_byte_matches_asm_modulo_registers() {
    let reference = include_str!("../../../examples/oob_smbus_msg.s");
    let got = tamal_lang::compile(&oob_tam()).expect("compile .tam");
    let want = tamal_asm::assemble(reference).expect("assemble reference .s");
    assert_eq!(
        canon(&got),
        canon(&want),
        "the HLL OOB message must match the hand-written .s modulo registers"
    );
}

#[test]
fn oob_smbus_msg_lowers_to_the_expected_asm() {
    // Pins the inline expansion, the folded 0xB1 TX CRC (never typed), the
    // allocator's lowest-free-first numbering, and the gensym labels. The
    // counter is shared, so `wait_state` takes `__wait0` and the `expect` that
    // follows takes `__fail1`.
    let asm = tamal_lang::lower_to_asm(&oob_tam()).expect("lower .tam");
    let expected = "\
.globl _start
_start:
\tset_config controller, x1, sck20, alert_pin
\tcs_assert
\tput_byte 0x06
\tput_byte 0x21
\tput_byte 0x00
\tput_byte 0x04
\tput_byte 0x10
\tput_byte 0x00
\tput_byte 0x01
\tput_byte 0xAB
\tput_byte 0xB1
\ttar 2
__wait0:
\tcrc_reset
\tget_byte x1
\tli x2, 0x0F
\tbeq x1, x2, __wait0
\tget_byte x1
\tget_byte x2
\tget_byte x3
\trdsr x3, crc
\tcs_deassert
\tbnez x3, __fail1
\thalt 0x00
__fail1:
\thalt 0x11
";
    assert_eq!(asm, expected);
}

#[test]
fn peripheral_io_read_via_fn_and_proc_byte_matches_asm_modulo_registers() {
    let reference = include_str!("../../../examples/peripheral_io_read.s");
    let got = tamal_lang::compile(&peripheral_tam()).expect("compile .tam");
    let want = tamal_asm::assemble(reference).expect("assemble reference .s");
    assert_eq!(
        canon(&got),
        canon(&want),
        "the same `command` proc must serve the peripheral read too"
    );
}

#[test]
fn peripheral_io_read_via_fn_and_proc_lowers_to_the_expected_asm() {
    // `iord_hdr(0x44, 0x0064)` folds to [0x44, 0x00, 0x64] and `+ crc8` to
    // 0x16; `repeat 1 { recv _ }` contributes the single payload read.
    //
    // **Not pinned here:** 0x0064's high byte is zero, so this golden cannot
    // tell `hi(addr)` from a constant `0` — that is pinned by
    // `consteval::tests::len_lo_hi_builtins`. The address is the 8042 status
    // port hard-coded in `examples/peripheral_io_read.s`, which `frames.rs`
    // byte-matches too, so it cannot be changed here alone.
    let asm = tamal_lang::lower_to_asm(&peripheral_tam()).expect("lower .tam");
    let expected = "\
.globl _start
_start:
\tset_config controller, x1, sck20, alert_pin
\tcs_assert
\tput_byte 0x44
\tput_byte 0x00
\tput_byte 0x64
\tput_byte 0x16
\ttar 2
__wait0:
\tcrc_reset
\tget_byte x1
\tli x2, 0x0F
\tbeq x1, x2, __wait0
\tget_byte x1
\tget_byte x1
\tget_byte x2
\tget_byte x3
\trdsr x3, crc
\tcs_deassert
\tbnez x3, __fail1
\thalt 0x00
__fail1:
\thalt 0x11
";
    assert_eq!(asm, expected);
}
