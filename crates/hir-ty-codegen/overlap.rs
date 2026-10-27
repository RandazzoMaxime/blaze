//! The locals of a body whose storage never overlaps share memory in the frame, as LLVM's
//! stack colouring has them share it from the lifetime markers rustc writes. Cranelift gives
//! every slot room of its own, so a function that matches on many shapes, each arm with its
//! temporaries, had a frame of all of them at once: hir-ide's `<Ty as HirDisplay>::hir_fmt`
//! had one of 20 KB, and its tests that print a type nested two hundred deep overflowed a
//! test thread's stack of 2 MB.
//!
//! Run on the function as [`promote`](super::promote) leaves it, just before Cranelift: a
//! local's slot that nothing addresses any more takes no room at all, and the rest are put
//! together greedily, the largest first, into one slot per group of locals of which no two
//! conflict ([`MirBody::frame_storage_conflicts`](hir_ty::mir::MirBody::frame_storage_conflicts)).

use cranelift_codegen::ir::{self, InstructionData};
use rustc_hash::FxHashMap;
use rustc_index::bit_set::BitMatrix;

use super::fragment::Named;

/// The slots of a body's locals, and which of them may hold a value at the same time.
pub(super) struct FrameLocals {
    /// `None` for a local kept in a register that never needed memory.
    pub(super) slots: Vec<Option<ir::StackSlot>>,
    /// By index in `slots`.
    pub(super) conflicts: BitMatrix<usize, usize>,
    /// Where in `slots` a local kept in a register puts the slot it gets when it needs one.
    pub(super) lazy: FxHashMap<hir_ty::mir::LocalId, usize>,
}

/// Shares the slots of `frame` between locals that never hold storage at once, and has
/// `named` say where each named local now is.
pub(super) fn share(func: &mut ir::Function, frame: &FrameLocals, named: &mut [Named]) {
    if frame.slots.is_empty() {
        return;
    }
    // An address of a slot that nothing uses is what `promote` left of its loads and stores.
    let mut used = rustc_hash::FxHashSet::default();
    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            used.extend(func.dfg.inst_values(inst).map(|it| func.dfg.resolve_aliases(it)));
        }
    }
    let mut addressed = vec![false; func.sized_stack_slots.len()];
    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            let Some(slot) = slot_of(&func.dfg.insts[inst]) else { continue };
            if used.contains(&func.dfg.first_result(inst)) {
                addressed[slot.as_u32() as usize] = true;
            }
        }
    }
    for it in named.iter() {
        addressed[it.slot.as_u32() as usize] = true;
    }
    let mut order: Vec<usize> = Vec::new();
    for (at, slot) in frame.slots.iter().enumerate() {
        let Some(slot) = *slot else { continue };
        if addressed[slot.as_u32() as usize] {
            order.push(at);
        } else {
            func.sized_stack_slots[slot].size = 0;
        }
    }
    let slot = |at: usize| frame.slots[at].expect("a local's slot");
    let size = |func: &ir::Function, at: usize| func.sized_stack_slots[slot(at)].size;
    order.sort_by_key(|&at| (std::cmp::Reverse(size(func, at)), at));
    // Each group's members, the first of them the slot the group keeps.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for at in order {
        let fits = |group: &Vec<usize>| group.iter().all(|&it| !frame.conflicts.contains(it, at));
        match groups.iter_mut().find(|group| fits(group)) {
            Some(group) => group.push(at),
            None => groups.push(vec![at]),
        }
    }
    let mut moved: FxHashMap<ir::StackSlot, ir::StackSlot> = FxHashMap::default();
    for group in groups.iter().filter(|it| it.len() > 1) {
        let kept = slot(group[0]);
        for &at in &group[1..] {
            let slot = slot(at);
            let (size, align_shift) =
                (func.sized_stack_slots[slot].size, func.sized_stack_slots[slot].align_shift);
            let into = &mut func.sized_stack_slots[kept];
            into.size = into.size.max(size);
            into.align_shift = into.align_shift.max(align_shift);
            func.sized_stack_slots[slot].size = 0;
            moved.insert(slot, kept);
        }
    }
    if moved.is_empty() {
        return;
    }
    let blocks: Vec<ir::Block> = func.layout.blocks().collect();
    for block in blocks {
        let insts: Vec<ir::Inst> = func.layout.block_insts(block).collect();
        for inst in insts {
            if let Some(slot) = slot_mut(&mut func.dfg.insts[inst])
                && let Some(&kept) = moved.get(slot)
            {
                *slot = kept;
            }
        }
    }
    for it in named.iter_mut() {
        if let Some(&kept) = moved.get(&it.slot) {
            it.slot = kept;
        }
    }
}

/// The slot an instruction addresses: only `stack_addr` names one, `stack_load` and
/// `stack_store` being built of it.
fn slot_of(inst: &InstructionData) -> Option<ir::StackSlot> {
    match *inst {
        InstructionData::StackAddr { stack_slot, .. } => Some(stack_slot),
        _ => None,
    }
}

fn slot_mut(inst: &mut InstructionData) -> Option<&mut ir::StackSlot> {
    match inst {
        InstructionData::StackAddr { stack_slot, .. } => Some(stack_slot),
        _ => None,
    }
}
