//! Register allocation: bind named runtime variables and scratch temporaries to
//! the tamal ISA's 15 usable physical registers (`x1`..`x15`; `x0` is the wired
//! zero and is never allocated). A scoped, lowest-free-first pool: entering a
//! scope (a `frame`, or an inlined `proc` expansion) and leaving it frees
//! everything bound inside it, so a nested scope can never clobber an outer
//! live value. A value that outlives its scope — an `expect`'s CRC residue,
//! latched inside an expansion but branched on at the frame's exit — is carried
//! across by [`RegAlloc::reserve`]. There is no spill target, so exhaustion is
//! a hard error (spec D5, §7).

use std::collections::HashMap;
use tamal_abi::isa::Reg;
use tamal_asm::{Diagnostic, Span};

/// The number of physical registers (`x0`..`x15`); `x0` is reserved (zero).
const NUM_REGS: usize = 16;

/// One open lexical scope: the registers allocated in it (freed on exit) and
/// the names bound in it (dropped from the environment on exit).
#[derive(Default)]
struct Scope {
    regs: Vec<Reg>,
    names: Vec<String>,
}

/// A scoped, lowest-free-first register allocator over `x1`..`x15`.
///
/// `x0` is never allocated. `exit_scope` frees everything allocated since the
/// matching `enter_scope`. Exhaustion is a hard error — there is no spill.
pub struct RegAlloc {
    /// `busy[n]` is true when physical register `xn` is in use.
    busy: [bool; NUM_REGS],
    /// The scope stack; index 0 is the root (test) scope.
    scopes: Vec<Scope>,
    /// Every live name binding → its physical register.
    bindings: HashMap<String, Reg>,
}

impl RegAlloc {
    /// A fresh allocator with a single root scope and nothing allocated.
    pub fn new() -> Self {
        RegAlloc {
            busy: [false; NUM_REGS],
            scopes: vec![Scope::default()],
            bindings: HashMap::new(),
        }
    }

    /// Open a nested scope (a `frame`, or a `proc` expansion); its allocations
    /// are released by the matching [`RegAlloc::exit_scope`].
    pub fn enter_scope(&mut self) {
        self.scopes.push(Scope::default());
    }

    /// Close the innermost scope, freeing its registers and dropping its names.
    /// The root scope is never popped, so an unbalanced `exit_scope` is a no-op
    /// rather than leaving the allocator scopeless.
    pub fn exit_scope(&mut self) {
        if self.scopes.len() <= 1 {
            return;
        }
        if let Some(scope) = self.scopes.pop() {
            for reg in scope.regs {
                self.busy[reg.bits() as usize] = false;
            }
            for name in scope.names {
                self.bindings.remove(&name);
            }
        }
    }

    /// Allocate the lowest free physical register, recording it in the current
    /// scope. Errors (no spill) when `x1`..`x15` are all in use.
    fn alloc(&mut self, span: &Span) -> Result<Reg, Diagnostic> {
        for n in 1..NUM_REGS {
            if !self.busy[n] {
                self.busy[n] = true;
                let reg = Reg::new(n as u8).expect("n < 16 fits a 5-bit register");
                self.scopes
                    .last_mut()
                    .expect("there is always a root scope")
                    .regs
                    .push(reg);
                return Ok(reg);
            }
        }
        Err(Diagnostic::error(
            span.clone(),
            "out of registers (the tamal ISA has only x1..x15 and no spill)",
        )
        .with_help("free a value by ending its scope, or use fewer live variables"))
    }

    /// Allocate an anonymous scratch register (freed by [`RegAlloc::free`] or at
    /// scope exit).
    pub fn temp(&mut self, span: &Span) -> Result<Reg, Diagnostic> {
        self.alloc(span)
    }

    /// Allocate a register and bind `name` to it for the current scope.
    pub fn bind(&mut self, name: String, span: &Span) -> Result<Reg, Diagnostic> {
        let reg = self.alloc(span)?;
        self.bindings.insert(name.clone(), reg);
        self.scopes
            .last_mut()
            .expect("there is always a root scope")
            .names
            .push(name);
        Ok(reg)
    }

    /// The register a name is currently bound to, if any.
    pub fn lookup(&self, name: &str) -> Option<Reg> {
        self.bindings.get(name).copied()
    }

    /// Release a register early (before its scope ends). Intended for anonymous
    /// `temp` scratch registers; named `bind`ings are released at scope exit.
    /// Idempotent and panic-free: freeing a register twice, one already released
    /// by `exit_scope`, or one outside the `x1`..`x15` window is harmless.
    pub fn free(&mut self, reg: Reg) {
        if let Some(slot) = self.busy.get_mut(reg.bits() as usize) {
            *slot = false;
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.regs.retain(|&r| r != reg);
        }
    }

    /// Re-take `reg` in the **current** scope, for a value whose allocating
    /// scope has ended but which is still live.
    ///
    /// The case this exists for: an `expect` inside an inlined `proc` latches
    /// the RX residue into a register owned by the expansion's scope, but the
    /// verdict branches on it at the enclosing `frame`'s exit. Without this the
    /// register would look free and a later statement could clobber the value
    /// before it is read. Idempotent.
    pub fn reserve(&mut self, reg: Reg) {
        if let Some(slot) = self.busy.get_mut(reg.bits() as usize) {
            *slot = true;
        }
        if let Some(scope) = self.scopes.last_mut() {
            if !scope.regs.contains(&reg) {
                scope.regs.push(reg);
            }
        }
    }
}

impl Default for RegAlloc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: u8) -> Reg {
        Reg::new(n).unwrap()
    }

    #[test]
    fn binds_lowest_free_first_and_reuses_after_free() {
        let mut a = RegAlloc::new();
        assert_eq!(a.bind("first".into(), &(0..0)).unwrap(), r(1));
        assert_eq!(a.bind("second".into(), &(0..0)).unwrap(), r(2));
        a.free(r(1));
        // x1 is free again, so the next temp takes it.
        assert_eq!(a.temp(&(0..0)).unwrap(), r(1));
    }

    #[test]
    fn lookup_finds_bound_names() {
        let mut a = RegAlloc::new();
        let reg = a.bind("data".into(), &(0..0)).unwrap();
        assert_eq!(a.lookup("data"), Some(reg));
        assert_eq!(a.lookup("nope"), None);
    }

    #[test]
    fn exit_scope_frees_regs_and_names() {
        let mut a = RegAlloc::new();
        a.enter_scope();
        a.bind("inner".into(), &(0..0)).unwrap(); // x1
        a.bind("inner2".into(), &(0..0)).unwrap(); // x2
        a.exit_scope();
        assert_eq!(a.lookup("inner"), None);
        // both x1 and x2 are free again
        assert_eq!(a.temp(&(0..0)).unwrap(), r(1));
        assert_eq!(a.temp(&(0..0)).unwrap(), r(2));
    }

    #[test]
    fn never_allocates_x0_and_caps_at_15() {
        let mut a = RegAlloc::new();
        let mut regs = Vec::new();
        for _ in 0..15 {
            regs.push(a.temp(&(0..0)).unwrap());
        }
        // x1..x15, never x0
        assert_eq!(
            regs.iter().map(|r| r.bits()).collect::<Vec<_>>(),
            (1..=15).collect::<Vec<u8>>()
        );
        // the 16th allocation has no register left: a hard error, no spill.
        let err = a.temp(&(0..0)).unwrap_err();
        assert!(err.message.contains("out of registers"));
    }

    #[test]
    fn nested_scope_does_not_clobber_a_live_outer_register() {
        // The module's headline invariant: a nested scope never reuses a live
        // outer register.
        let mut a = RegAlloc::new();
        let outer = a.temp(&(0..0)).unwrap(); // x1, stays live across the scope
        a.enter_scope();
        let inner = a.temp(&(0..0)).unwrap();
        assert_ne!(outer, inner);
        assert_eq!(inner, r(2));
        a.exit_scope();
        // inner (x2) is reclaimed; outer (x1) is still live, so the next temp is
        // x2 again — never x1.
        assert_eq!(a.temp(&(0..0)).unwrap(), r(2));
    }

    #[test]
    fn reserve_retakes_a_register_whose_scope_has_ended() {
        // The `expect`-inside-a-`proc` case: a value is latched in a nested
        // scope but read after that scope ends, so the register must be re-taken
        // or a later allocation would clobber it.
        let mut a = RegAlloc::new();
        a.enter_scope();
        let latched = a.temp(&(0..0)).unwrap(); // x1, owned by the inner scope
        a.exit_scope(); // x1 would now be free …
        a.reserve(latched); // … but the value is still live
        assert_eq!(a.temp(&(0..0)).unwrap(), r(2), "must not hand out x1 again");
    }

    #[test]
    fn a_reserved_register_is_freed_by_the_scope_that_reserved_it() {
        // `reserve` re-takes the register into the CURRENT scope, which is the
        // one that will consume the value (the `frame`, for a verdict latched
        // inside an inlined `proc`). Marking it busy without recording an owner
        // would leak it for the rest of the program.
        let mut a = RegAlloc::new();
        a.enter_scope(); // the frame
        a.enter_scope(); // the proc expansion
        let latched = a.temp(&(0..0)).unwrap(); // x1
        a.exit_scope(); // the expansion ends …
        a.reserve(latched); // … but the frame still needs the value
        a.exit_scope(); // the frame ends: now x1 really is free
        assert_eq!(a.temp(&(0..0)).unwrap(), r(1), "x1 must be reclaimed");
    }

    #[test]
    fn freeing_an_unissued_or_already_free_register_is_harmless() {
        let mut a = RegAlloc::new();
        a.free(r(1)); // never allocated -> no-op
        a.free(Reg::new(20).unwrap()); // outside the x1..x15 window -> must NOT panic
        // the allocator is unaffected and still starts at x1
        assert_eq!(a.temp(&(0..0)).unwrap(), r(1));
    }
}
