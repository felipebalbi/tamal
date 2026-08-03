//! Shared test helper: compare two programs **modulo register allocation**.
//!
//! Decode each assembled word back to a `tamal_abi::isa::Instr` and zero every
//! register operand, so two programs compare equal when they differ only in
//! which physical registers the allocator happened to choose — while the
//! opcodes, immediates, folded CRC bytes and branch offsets still have to match
//! exactly (spec §9).

use tamal_abi::isa::{Instr, Reg};

/// Decode a program and zero every register operand.
pub fn canon(prog: &tamal_asm::Program) -> Vec<Instr> {
    prog.words()
        .map(|w| zero_regs(Instr::decode(w).expect("re-decode assembled word")))
        .collect()
}

fn zero_regs(i: Instr) -> Instr {
    use Instr::*;
    let z = Reg::new(0).unwrap();
    match i {
        PutByteReg(_) => PutByteReg(z),
        GetByte(_) => GetByte(z),
        PutBitsReg(_, n) => PutBitsReg(z, n),
        GetBits(_, n) => GetBits(z, n),
        TarReg(_) => TarReg(z),
        GetAlert(_) => GetAlert(z),
        Beq(_, _, o) => Beq(z, z, o),
        Bne(_, _, o) => Bne(z, z, o),
        Bltu(_, _, o) => Bltu(z, z, o),
        Bgeu(_, _, o) => Bgeu(z, z, o),
        WaitOn(_, c, t) => WaitOn(z, c, t),
        Mark(tag, _) => Mark(tag, z),
        LoadImm(_, v) => LoadImm(z, v),
        Lui(_, v) => Lui(z, v),
        Mov(_, _) => Mov(z, z),
        Add(_, _, _) => Add(z, z, z),
        Addi(_, _, o) => Addi(z, z, o),
        Sub(_, _, _) => Sub(z, z, z),
        And(_, _, _) => And(z, z, z),
        Andi(_, _, o) => Andi(z, z, o),
        Or(_, _, _) => Or(z, z, z),
        Ori(_, _, o) => Ori(z, z, o),
        Xor(_, _, _) => Xor(z, z, z),
        Xori(_, _, o) => Xori(z, z, o),
        Shift(_, _, op, a) => Shift(z, z, op, a),
        Rdsr(_, sr) => Rdsr(z, sr),
        // register-free instructions are unchanged
        other => other,
    }
}
