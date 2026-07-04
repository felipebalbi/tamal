//! Disassembler: little-endian bytecode -> a textual listing.

use tamal_abi::isa::Instr;

use crate::mnemonics::render_instr;

/// Disassemble little-endian bytecode into an `addr  word  text` listing.
/// A word that does not decode is shown with its `DecodeError`.
pub fn disassemble(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut addr: u16 = 0;
    let mut chunks = bytes.chunks_exact(4);
    for c in &mut chunks {
        let w = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        match Instr::decode(w) {
            Ok(i) => out.push_str(&format!("{addr:04x}  {w:08x}  {}\n", render_instr(&i))),
            Err(e) => out.push_str(&format!("{addr:04x}  {w:08x}  ; {e:?}\n")),
        }
        addr = addr.wrapping_add(1);
    }
    let rem = chunks.remainder();
    if !rem.is_empty() {
        out.push_str(&format!(
            "; trailing {} byte(s): not a whole 32-bit word\n",
            rem.len()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tamal_abi::isa::Instr;

    #[test]
    fn disassembles_words_and_flags_illegal() {
        let mut bytes = Instr::CsAssert.encode().to_le_bytes().to_vec();
        bytes.extend_from_slice(&Instr::Halt(0x00).encode().to_le_bytes());
        bytes.extend_from_slice(&0xC000_0000u32.to_le_bytes()); // reserved group -> illegal
        let text = disassemble(&bytes);
        assert!(text.contains("cs_assert"));
        assert!(text.contains("halt 0"));
        assert!(text.contains("IllegalOpcode"));
    }
}
