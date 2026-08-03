//! End-to-end: one `proc` — the body Plan 5 will move into the bundled `espi`
//! stdlib — drives all four eSPI channels spec §9 names, and each lowers to
//! bytecode byte-identical *modulo register allocation* to its hand-written
//! `.s`. The four differ **only** in their command-phase bytes; the framing,
//! WAIT_STATE poll and CRC verdict come from the single shared `command`.

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

/// Virtual Wire channel: deassert PLTRST# by sending one VW index/data pair.
/// The ACCEPT completion carries status only, so `ndata = 0`; arguments named.
///
/// **Not pinned here**, exactly as for the OOB case above: the named arguments
/// are written in declaration order, so this cannot tell name-matching from
/// position-matching — verified by mutation (binding named arguments by index
/// survives all four channel tests, and is killed by
/// `consteval::tests::{binds_named_arguments_by_name,
/// rejects_a_named_argument_with_no_matching_parameter,
/// rejects_a_parameter_bound_twice}`).
fn virtual_wire_tam() -> String {
    format!(
        "const PUT_VWIRE = 0x04\n\
const VW_INDEX_SYS = 0x03\n\
const VW_PLTRST_HIGH = 0x22\n\n{COMMAND_PROC}\n\
test vwire_pltrst {{
    config controller, x1, sck20, alert_pin
    command(
        pkt = [PUT_VWIRE, 0x00, VW_INDEX_SYS, VW_PLTRST_HIGH],
        ndata = 0,
    )
    pass
}}
"
    )
}

/// Flash Access channel: return a 4-byte read completion. The four data bytes
/// ride in the *command* phase (this is the completion, not a request), so the
/// response is status-only and `ndata = 0` again — passed positionally.
fn flash_completion_tam() -> String {
    format!(
        "const PUT_FLASH_C = 0x08\n\
const CYCLE_SCMPL_D = 0x09\n\n{COMMAND_PROC}\n\
test flash_cmpl {{
    config controller, x1, sck20, alert_pin
    command([PUT_FLASH_C, CYCLE_SCMPL_D, 0x00, 0x04,
             0xDE, 0xAD, 0xBE, 0xEF], 0)
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

#[test]
fn virtual_wire_pltrst_byte_matches_asm_modulo_registers() {
    let reference = include_str!("../../../examples/virtual_wire_pltrst.s");
    let got = tamal_lang::compile(&virtual_wire_tam()).expect("compile .tam");
    let want = tamal_asm::assemble(reference).expect("assemble reference .s");
    assert_eq!(
        canon(&got),
        canon(&want),
        "the same `command` proc must serve the virtual-wire drive too"
    );
}

#[test]
fn virtual_wire_pltrst_lowers_to_the_expected_asm() {
    // Pins the folded 0x89 TX CRC — never typed in the `.tam`, and re-derived
    // here from the four command bytes rather than copied off the reference
    // `.s`. `ndata = 0`, so the unroll contributes no `get_byte` at all and the
    // response phase is the bare status pair + CRC residue.
    let asm = tamal_lang::lower_to_asm(&virtual_wire_tam()).expect("lower .tam");
    let expected = "\
.globl _start
_start:
\tset_config controller, x1, sck20, alert_pin
\tcs_assert
\tput_byte 0x04
\tput_byte 0x00
\tput_byte 0x03
\tput_byte 0x22
\tput_byte 0x89
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
fn flash_completion_byte_matches_asm_modulo_registers() {
    let reference = include_str!("../../../examples/flash_completion.s");
    let got = tamal_lang::compile(&flash_completion_tam()).expect("compile .tam");
    let want = tamal_asm::assemble(reference).expect("assemble reference .s");
    assert_eq!(
        canon(&got),
        canon(&want),
        "the same `command` proc must serve the flash completion too"
    );
}

#[test]
fn flash_completion_lowers_to_the_expected_asm() {
    // The longest of the four command phases: eight bytes plus the folded 0xE8
    // TX CRC over all eight. The response phase is byte-for-byte the same as
    // the other three — which is the whole point of the shared `command`.
    let asm = tamal_lang::lower_to_asm(&flash_completion_tam()).expect("lower .tam");
    let expected = "\
.globl _start
_start:
\tset_config controller, x1, sck20, alert_pin
\tcs_assert
\tput_byte 0x08
\tput_byte 0x09
\tput_byte 0x00
\tput_byte 0x04
\tput_byte 0xDE
\tput_byte 0xAD
\tput_byte 0xBE
\tput_byte 0xEF
\tput_byte 0xE8
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
