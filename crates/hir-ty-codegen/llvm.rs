//! LLVM as the producer of a function's machine code.
//!
//! 1. [`lower`] turns the Cranelift IR of the function into an LLVM function of a module of
//!    its own. What the function refers to is declared there under the names of [`Symbol`];
//!    the bodies of callees LLVM is shown are functions beside it ([`shown_name`]), and on
//!    Mach-O the function is built into a [`wrap`]per.
//! 2. LLVM optimizes the module, if asked to, the bodies shown give way to the functions
//!    they are of ([`unshow`]), and LLVM writes an object file into memory.
//! 3. [`emitted`] reads the object back: the code, the constants LLVM put beside it, the
//!    relocations, with their [`Symbol`]s turned back into requests, and the unwind table.
//!
//! Anything on the way that this has no answer for is an [`Unhandled`], and the function is
//! compiled by Cranelift instead.

use std::{cell::RefCell, collections::hash_map::Entry, fmt, sync::Once};

use cranelift_codegen::ir::{self, Opcode, Type};
use inkwell::{
    OptimizationLevel,
    attributes::{Attribute, AttributeLoc},
    context::Context,
    llvm_sys::support::LLVMParseCommandLineOptions,
    memory_buffer::MemoryBuffer,
    module::{Linkage, Module},
    passes::PassBuilderOptions,
    targets::{
        CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
    },
    values::FunctionValue,
};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    Arch,
    backend::{Optimization, Passes, Phase, Selection, Tally},
    fragment::{Code, Hint, Wants},
    imports::Import,
};

mod emitted;
mod lower;
mod rebase;

/// What runs again where [`rebase::rebase`] said what addresses are based on: what is stored
/// through one and loaded again is forwarded, and a slot that is then only written goes.
const REBASED: &str = "function(instcombine<no-verify-fixpoint>,sroa,gvn,dse,\
                       instcombine<no-verify-fixpoint>,simplifycfg)";

use lower::{Defined, Lowering};

/// The name of the function being compiled, in the module and in the object.
const OWN: &str = "ra-jit.own";
const PERSONALITY: &str = "rust_eh_personality";

/// Whether every call of `func` that unwinds has one handler, which takes the exception alone.
/// A call of a body Cranelift built in has handlers of its own and of the caller's, which
/// may take the caller's values; LLVM lowers those too, each with a landing pad of its own.
pub(super) fn plain_handlers(func: &ir::Function) -> bool {
    let dfg = &func.dfg;
    dfg.exception_tables.values().all(|table| {
        let mut items = table.items();
        match (items.next(), items.next()) {
            (Some(ir::ExceptionTableItem::Tag(_, handler)), None) => {
                matches!(
                    handler.args(&dfg.value_lists).collect::<Vec<_>>()[..],
                    [ir::BlockArg::TryCallExn(0)]
                )
            }
            _ => false,
        }
    })
}

/// `named` are the slots of the locals with a name. `weights`, from what code of the function
/// counted while it ran, become its `!prof`. `imports` are the bodies of what it calls that
/// LLVM may build in ([`super::imports`]).
pub(super) fn emit(
    recorder: &mut Wants,
    named: &[ir::StackSlot],
    func: &ir::Function,
    optimization: Optimization,
    weights: Option<&super::tier::Weights>,
    imports: &[Import],
) -> Result<Code, Unhandled> {
    let tally = Tally::global();
    // LLVM's optimizing register allocator takes more than linear time, as Cranelift's does:
    // one function of dx kept it busy for minutes. Such a function is compiled without
    // optimization, and counted.
    let optimization = if func.dfg.num_insts() > super::LARGE && optimization != Optimization::NONE
    {
        tally.large.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Optimization::NONE
    } else {
        optimization
    };
    let seen = std::mem::take(&mut recorder.seen);
    // Without a line table to keep (see `lower::Marks`), and where a parameter is the only
    // way to its memory.
    let wrapped = optimization.pipeline().is_some()
        && recorder.pointees.iter().any(|(_, it)| it.unique)
        && super::Target::selected().is_ok_and(|it| it.object_format == super::ObjectFormat::MachO);
    Machine::with(optimization, |context, machine| {
        let module = context.create_module("ra-jit");
        module.set_triple(&machine.get_triple());
        module.set_data_layout(&machine.get_target_data().get_data_layout());
        let constants: FxHashMap<u32, &[u8]> = recorder.constants().collect();
        let statics: FxHashSet<u32> = recorder.statics().collect();
        let resume = recorder.resume();
        let memcmp = recorder.memcmp();
        let shown = tally.time(Phase::Lower, || {
            let own = Defined {
                name: if wrapped { INNER } else { OWN },
                func,
                pointees: &recorder.pointees,
                named,
                weights,
                marked: true,
            };
            let names: Vec<String> = seen.iter().map(|it| shown_name(it.id.as_u32())).collect();
            let shown: Vec<Defined<'_>> = std::iter::zip(&seen, &names)
                .map(|(it, name)| Defined {
                    name,
                    func: &it.func,
                    pointees: &it.pointees,
                    named: &[],
                    weights: None,
                    marked: false,
                })
                .collect();
            let function = Lowering::add(context, &module, &own)?;
            // Before the function's body: its calls of what is imported are of the bodies.
            for import in imports {
                link(context, &module, import)?;
            }
            // Every body shown is declared before any is defined, as one may call another.
            let mut by_id = FxHashMap::default();
            for (it, defined) in std::iter::zip(&seen, &shown) {
                if let Ok(function) = Lowering::add(context, &module, defined) {
                    mark_hint(context, function, it.hint);
                    by_id.insert(it.id.as_u32(), function);
                }
            }
            // A body shown calls the others shown, as the function being compiled does, so
            // that what LLVM learns of one (that it keeps no pointer it is given, say) holds
            // of those that call it.
            let declared = by_id.clone();
            for (it, defined) in std::iter::zip(&seen, &shown) {
                let Some(&function) = by_id.get(&it.id.as_u32()) else { continue };
                let defined = Lowering::define(
                    context, &module, function, defined, &constants, &statics, resume, memcmp,
                    &declared,
                );
                if defined.is_err() {
                    by_id.remove(&it.id.as_u32());
                    unshow(&module, &FxHashMap::from_iter([(it.id.as_u32(), function)]));
                }
            }
            Lowering::define(
                context, &module, function, &own, &constants, &statics, resume, memcmp, &by_id,
            )?;
            if wrapped {
                wrap(context, &module, function, weights)?;
            }
            Ok::<_, Unhandled>(by_id)
        })?;
        // `RA_JIT_DUMP_LLVM=after` prints the IR as the passes left it, anything else as it
        // goes in.
        let dump = std::env::var("RA_JIT_DUMP_LLVM").ok();
        if dump.as_deref().is_some_and(|it| it != "after") {
            eprintln!("{}", module.print_to_string().to_string_lossy());
        }
        // LLVM aborts the process on IR it does not accept, so it is checked first.
        tally
            .time(Phase::Verify, || module.verify())
            .map_err(|e| Unhandled::Llvm(e.to_string_lossy().into_owned()))?;
        if let Some(pipeline) = optimization.pipeline() {
            tally
                .time(Phase::Optimize, || {
                    module.run_passes(pipeline, machine, PassBuilderOptions::create())
                })
                .map_err(|e| Unhandled::Llvm(e.to_string_lossy().into_owned()))?;
            if rebase::rebase(&module) {
                tally
                    .time(Phase::Optimize, || {
                        module.run_passes(REBASED, machine, PassBuilderOptions::create())
                    })
                    .map_err(|e| Unhandled::Llvm(e.to_string_lossy().into_owned()))?;
            }
        }
        // What is imported is only there to be built in: a body LLVM did not build in is
        // the callee's own code, called. The passes above drop them; a function too large
        // for them gets none.
        if !imports.is_empty() {
            module
                .run_passes("elim-avail-extern", machine, PassBuilderOptions::create())
                .map_err(|e| Unhandled::Llvm(e.to_string_lossy().into_owned()))?;
        }
        unshow(&module, &shown);
        if module.get_function(INNER).is_some() {
            return Err(Unhandled::Shape("a body not built into the function that calls it"));
        }
        if dump.as_deref() == Some("after") {
            eprintln!("{}", module.print_to_string().to_string_lossy());
        }
        let object = tally
            .time(Phase::Emit, || machine.write_to_memory_buffer(&module, FileType::Object))
            .map_err(|e| Unhandled::Llvm(e.to_string_lossy().into_owned()))?;
        tally.time(Phase::Read, || {
            emitted::Emitted::code(object.as_slice(), func.signature.clone(), recorder)
        })
    })
}

/// The function being compiled, when it is [`wrap`]ped.
const INNER: &str = "ra-jit.inner";

/// Has the function being compiled, `inner`, called by one that only calls it, under the
/// name [`OWN`], with the same parameters, and built into that one.
///
/// LLVM builds a function into its caller by telling each access through a `noalias`
/// parameter from those that cannot be through it (`!alias.scope`, `!noalias`): a call of
/// a function that is not passed the address cannot reach the memory if the address did
/// not escape before the call, which it works out along the paths through the function.
/// Its loop passes, which take an address that escapes anywhere in the function to escape
/// everywhere, know then that a call in a loop of `&mut self`'s fields leaves the fields
/// alone when `self` is passed on only after the loop.
fn wrap<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    inner: FunctionValue<'ctx>,
    weights: Option<&super::tier::Weights>,
) -> Result<(), Unhandled> {
    let outer = module.add_function(OWN, inner.get_type(), Some(Linkage::External));
    let places = (0..inner.count_params()).map(AttributeLoc::Param);
    for loc in [AttributeLoc::Function, AttributeLoc::Return].into_iter().chain(places) {
        for attribute in inner.attributes(loc) {
            outer.add_attribute(loc, attribute);
        }
    }
    if let Some(weights) = weights {
        lower::entry_count(context, outer, weights.entered);
    }
    inner.set_linkage(Linkage::Internal);
    let always = Attribute::get_named_enum_kind_id("alwaysinline");
    inner.add_attribute(AttributeLoc::Function, context.create_enum_attribute(always, 0));
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(outer, ""));
    let args: Vec<_> = outer.get_param_iter().map(Into::into).collect();
    let call = builder.build_direct_call(inner, &args, "")?;
    match call.try_as_basic_value().basic() {
        Some(returned) => builder.build_return(Some(&returned))?,
        None => builder.build_return(None)?,
    };
    Ok(())
}

/// Tells LLVM what the function's `#[inline]` asks, as rustc does.
fn mark_hint<'ctx>(context: &'ctx Context, function: FunctionValue<'ctx>, hint: Hint) {
    let hint = match hint {
        Hint::None => return,
        Hint::Inline => "inlinehint",
        Hint::Always => "alwaysinline",
        Hint::Never => "noinline",
    };
    let hint = Attribute::get_named_enum_kind_id(hint);
    function.add_attribute(AttributeLoc::Function, context.create_enum_attribute(hint, 0));
}

/// The passes an export is simplified by before compiles import it: ThinLTO's pre-link
/// pipeline, which leaves loops to be unrolled and vectorized where the body is built in,
/// after the slots are taken apart as for a compile ([`Optimization::pipeline`]).
fn exported(optimization: Optimization) -> &'static str {
    match optimization.passes {
        Passes::Aggressive => {
            "function(sroa,instsimplify,sroa,jump-threading),thinlto-pre-link<O3>"
        }
        _ => "function(sroa,instsimplify,sroa,jump-threading),thinlto-pre-link<O2>",
    }
}

/// The export of a function for the compiles that call it ([`super::imports`]): its module
/// alone, with no line table nor variables, in bitcode once LLVM's pre-link passes are done
/// with it, and how many instructions it has then.
pub(super) fn export(
    recorder: &Wants,
    func: &ir::Function,
    pointees: &[(u32, super::fragment::Pointee)],
    hint: Hint,
    optimization: Optimization,
) -> Result<(Vec<u8>, usize), Unhandled> {
    if func.dfg.num_insts() > super::LARGE {
        return Err(Unhandled::Shape("a body too large to export"));
    }
    Machine::with(optimization, |context, machine| {
        let module = context.create_module("ra-jit");
        module.set_triple(&machine.get_triple());
        module.set_data_layout(&machine.get_target_data().get_data_layout());
        let constants: FxHashMap<u32, &[u8]> = recorder.constants().collect();
        let statics: FxHashSet<u32> = recorder.statics().collect();
        let own = Defined { name: OWN, func, pointees, named: &[], weights: None, marked: false };
        let function = Lowering::add(context, &module, &own)?;
        mark_hint(context, function, hint);
        let shown = FxHashMap::default();
        let (resume, memcmp) = (recorder.resume(), recorder.memcmp());
        Lowering::define(
            context, &module, function, &own, &constants, &statics, resume, memcmp, &shown,
        )?;
        module.verify().map_err(|e| Unhandled::Llvm(e.to_string_lossy().into_owned()))?;
        module
            .run_passes(exported(optimization), machine, PassBuilderOptions::create())
            .map_err(|e| Unhandled::Llvm(e.to_string_lossy().into_owned()))?;
        let size = function.get_basic_blocks().into_iter().map(|it| it.get_instructions().count());
        Ok((module.write_bitcode_to_memory().as_slice().to_vec(), size.sum()))
    })
}

/// Puts the body `import` has in `module`, `available_externally` under the name of the
/// request it is of, with what it refers to under the names of the compile's requests.
fn link<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    import: &Import,
) -> Result<(), Unhandled> {
    let llvm = |e: inkwell::support::LLVMString| Unhandled::Llvm(e.to_string_lossy().into_owned());
    let buffer = MemoryBuffer::create_from_memory_range(&import.export.bitcode, "import");
    let imported = context.create_module_from_ir(buffer).map_err(llvm)?;
    let own = imported.get_function(OWN).ok_or(Unhandled::Shape("an export without its body"))?;
    let own_name = Symbol::Func(import.id).to_string();
    let renamed = |name: &std::ffi::CStr| match Symbol::parse(name.to_str().ok()?)? {
        Symbol::Func(at) => Some(Symbol::Func(*import.funcs.get(at as usize)?).to_string()),
        Symbol::Data(at) => Some(Symbol::Data(*import.datas.get(at as usize)?).to_string()),
    };
    let mut names = vec![(own.as_global_value(), own_name.clone())];
    let functions: Vec<FunctionValue<'ctx>> = imported.get_functions().collect();
    for function in functions.into_iter().filter(|&it| it != own) {
        let Some(name) = renamed(function.get_name()) else { continue };
        // A call of itself is of the body.
        if name == own_name {
            function.replace_all_uses_with(own);
            // SAFETY: nothing refers to the declaration any more.
            unsafe { function.delete() };
            continue;
        }
        names.push((function.as_global_value(), name));
    }
    for global in imported.get_globals() {
        if let Some(name) = renamed(global.get_name()) {
            names.push((global, name));
        }
    }
    // In two steps, as a name given may be one another still has.
    for (at, (global, _)) in names.iter().enumerate() {
        global.set_name(&format!("ra-jit.import.{at}"));
    }
    for (global, name) in &names {
        global.set_name(name);
    }
    own.set_linkage(Linkage::AvailableExternally);
    // LLVM links in a body that is only available externally only where the module it goes
    // to has the function, and the compile's calls are lowered after.
    if module.get_function(&own_name).is_none() {
        module.add_function(&own_name, own.get_type(), Some(Linkage::External));
    }
    module.link_in_module(imported).map_err(llvm)
}

/// What the body of the requested function `id` that LLVM is shown is called in the module.
/// No symbol of a program has such a name.
fn shown_name(id: u32) -> String {
    format!("ra-jit.a.{id}")
}

/// Has every call of a body shown that LLVM kept call the function itself, and drops the
/// bodies: only the function being compiled is in the object. What LLVM made of the calls
/// from what it read in the bodies holds of the functions, which are what the bodies are of
/// (an edit of one sends the function being compiled back to level 1).
fn unshow<'ctx>(module: &Module<'ctx>, shown: &FxHashMap<u32, FunctionValue<'ctx>>) {
    for (&id, &body) in shown {
        let name = Symbol::Func(id).to_string();
        let function = module.get_function(&name).unwrap_or_else(|| {
            module.add_function(&name, body.get_type(), Some(Linkage::External))
        });
        let address = body.as_global_value().as_pointer_value();
        address.replace_all_uses_with(function.as_global_value().as_pointer_value());
        // SAFETY: nothing refers to the body any more.
        unsafe { body.delete() };
    }
}

/// The object file LLVM's assembler makes of `text`, which is assembly in the syntax
/// `asm!` is written in.
///
/// FIXME: assembly LLVM rejects ends the process: its default diagnostic handler exits on an
/// error, and inkwell has no way to set another.
pub(super) fn assemble(text: &str) -> Result<Vec<u8>, String> {
    Machine::with(Optimization::NONE, |context, machine| {
        let module = context.create_module("ra_jit_asm");
        module.set_triple(&machine.get_triple());
        // What rustc wraps `global_asm!` in, where there are two syntaxes.
        let text = match super::Target::selected().map_err(Unhandled::Llvm)?.arch {
            Arch::X86_64 => format!(".intel_syntax noprefix\n{text}\n.att_syntax\n"),
            Arch::Aarch64 | Arch::Wasm32 => format!("{text}\n"),
        };
        module.set_inline_assembly(&text);
        let object = machine
            .write_to_memory_buffer(&module, FileType::Object)
            .map_err(|e| Unhandled::Llvm(e.to_string_lossy().into_owned()))?;
        Ok(object.as_slice().to_vec())
    })
    .map_err(|e| format!("inline assembly: {e}"))
}

/// Sets the options of LLVM that only its command line reaches, for the process.
///
/// The SLP vectorizer is off. LLVM 20.1's makes one store of several equal rows of an array
/// of `i128` with lanes of it left as poison, so that part of the array is never written
/// (`breakers/cases/src/repeat_rows.rs`). The `default<O2>` pipeline turns the pass on
/// whatever its options say, which leaves the flag the pass itself reads.
///
/// On AArch64 the quick selection is not the global instruction selector. That one deletes
/// the unused copy of the selector register at a landing pad and leaves the register a
/// live-in of the block, and LLVM 20.1's fast register allocator does not reload a value it
/// has put in a live-in register: after optimization, which keeps values in registers across
/// an unwind edge, a landing pad then reads the selector where it wants a drop flag
/// (`breakers/cases/src/unwind_many_flags.rs`).
fn set_options(arch: Arch) {
    static SET: Once = Once::new();
    SET.call_once(|| {
        let mut arguments = vec![c"ra-jit".as_ptr(), c"-vectorize-slp=false".as_ptr()];
        if arch == Arch::Aarch64 {
            arguments.push(c"-aarch64-enable-global-isel-at-O=-1".as_ptr());
        }
        // SAFETY: the arguments are C strings that outlive the call, and LLVM is given
        // their number.
        unsafe {
            LLVMParseCommandLineOptions(
                arguments.len() as i32,
                arguments.as_ptr(),
                std::ptr::null(),
            )
        }
    });
}

impl Optimization {
    /// The passes to run. What `lower` writes keeps every value in a stack slot, and a slot
    /// indexed by a number that is itself in a slot is not taken apart until that number is
    /// known; left to the default pipeline, such a slot is deleted whole and the variable it
    /// holds is lost to the debugger. So the slots are taken apart twice first, with the
    /// constants folded in between.
    ///
    /// Then branches are threaded. A call Cranelift built in returns through a block that
    /// joins its returns, and the caller branches on what it returned there: an `Option`'s
    /// discriminant. Left to the default pipeline, the join is folded into `select`s first,
    /// and a loop over a range counts with `i + (i < n)`, which LLVM can neither unroll nor
    /// prove an index of in bounds or free of overflow with. Threaded, it counts with `i + 1`.
    ///
    /// At `-O3` the bodies LLVM is shown are then built in, and the same is done again to what
    /// they joined, with values numbered once after it. A body LLVM builds in itself returns
    /// through a join as well, and the default pipeline numbers values before it threads
    /// branches: a range's start the loop over it advances to, `i + 1`, stays two values when
    /// the body of the loop computes it too, until instruction combining makes the inner loop
    /// of `for j in i + 1..n` count with `phi + 1`. An index scaled from that is not known
    /// free of overflow, so alias analysis cannot tell `bodies[i].velocity` from
    /// `bodies[j].mass`: every iteration reloads what it stored (nbody, 1.97 times
    /// release on Graviton4, 1.28 with the bodies built in first).
    ///
    /// At `-O3` what LLVM made redundant is then removed once more. A loop of a callee
    /// Cranelift built in is unrolled late in the pipeline, after LLVM's last numbering of
    /// values, while the copies are still joined by branches on constants: each copy keeps
    /// its own `%` and overflow check (16 `urem`s where rustc, which optimizes the callee on
    /// its own before building it in, has 7, in the Life benchmark's `step`). Instruction
    /// combining named in a pipeline checks that one run of it left nothing to combine and
    /// aborts the process when it did, which the default pipelines do not ask for.
    fn pipeline(self) -> Option<&'static str> {
        match self.passes {
            Passes::None => None,
            Passes::Less => Some("function(sroa,instsimplify,sroa),default<O1>"),
            Passes::Default => Some("function(sroa,instsimplify,sroa,jump-threading),default<O2>"),
            Passes::Aggressive => Some(
                "function(sroa,instsimplify,sroa,jump-threading),\
                 cgscc(inline,function(sroa,instsimplify,sroa,jump-threading,early-cse<memssa>)),\
                 default<O3>,function(gvn,sccp,instcombine<no-verify-fixpoint>,simplifycfg)",
            ),
        }
    }

    /// The level of the code generator, which is not that of the passes.
    fn level(self) -> OptimizationLevel {
        match (self.selection, self.passes) {
            (Selection::Fast, _) | (_, Passes::None) => OptimizationLevel::None,
            (Selection::Optimized, Passes::Less) => OptimizationLevel::Less,
            (Selection::Optimized, Passes::Default) => OptimizationLevel::Default,
            (Selection::Optimized, Passes::Aggressive) => OptimizationLevel::Aggressive,
        }
    }
}

/// The target as LLVM knows it, one per thread and level: making one costs more than a
/// small function does. The context the modules are made in is kept with them, as a
/// machine holds on to things of the last context it compiled a module of: the instruction
/// selector of AArch64 keeps the source location of the last instruction it saw, and lets
/// go of it at the next function, by when a context of that one module would be gone.
#[derive(Default)]
struct Machine {
    // Before the context, which has to outlive them.
    machines: FxHashMap<Optimization, TargetMachine>,
    context: Option<Context>,
    /// How many modules were made in the context.
    modules: usize,
}

impl Machine {
    /// How many modules a context is kept for. The constants and source locations of all
    /// of them stay in it.
    const MODULES: usize = 1024;

    fn with<T>(
        optimization: Optimization,
        work: impl FnOnce(&Context, &TargetMachine) -> Result<T, Unhandled>,
    ) -> Result<T, Unhandled> {
        thread_local! {
            static MACHINE: RefCell<Machine> = RefCell::default();
        }
        MACHINE.with_borrow_mut(|this| {
            if this.modules == Self::MODULES {
                *this = Machine::default();
            }
            this.modules += 1;
            let machine = match this.machines.entry(optimization) {
                Entry::Occupied(known) => known.into_mut(),
                Entry::Vacant(slot) => slot.insert(Self::create(optimization)?),
            };
            work(this.context.get_or_insert_with(Context::create), machine)
        })
    }

    fn create(optimization: Optimization) -> Result<TargetMachine, Unhandled> {
        let emitted = super::Target::selected().map_err(Unhandled::Llvm)?;
        set_options(emitted.arch);
        match emitted.arch {
            Arch::X86_64 => Target::initialize_x86(&InitializationConfig::default()),
            Arch::Aarch64 => Target::initialize_aarch64(&InitializationConfig::default()),
            // The lowering of `wasm.rs` produces a module's functions.
            Arch::Wasm32 => return Err(Unhandled::Llvm("no LLVM producer for wasm32".to_owned())),
        }
        let name = emitted.triple();
        let triple = TargetTriple::create(name);
        let target = Target::from_triple(&triple)
            .map_err(|e| Unhandled::Llvm(e.to_string_lossy().into()))?;
        // The baseline CPU, as rustc compiles for; position-independent, as the rest of
        // the program is.
        target
            .create_target_machine(
                &triple,
                emitted.cpu(),
                "",
                optimization.level(),
                RelocMode::PIC,
                CodeModel::Default,
            )
            .ok_or_else(|| Unhandled::Llvm(format!("no target machine for {name}")))
    }
}

/// What a function or data object the code refers to is called in the module, and so in
/// the relocations of the object: the request with that index. No symbol of a program has
/// such a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Symbol {
    Func(u32),
    Data(u32),
}

impl Symbol {
    /// Cranelift's `User` names: namespace 0 holds the functions, 1 the data objects.
    fn of_user(namespace: u32, index: u32) -> Symbol {
        match namespace {
            0 => Symbol::Func(index),
            _ => Symbol::Data(index),
        }
    }

    fn parse(name: &str) -> Option<Symbol> {
        if let Some(index) = name.strip_prefix("ra-jit.f.") {
            return index.parse().ok().map(Symbol::Func);
        }
        name.strip_prefix("ra-jit.d.")?.parse().ok().map(Symbol::Data)
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Symbol::Func(index) => write!(f, "ra-jit.f.{index}"),
            Symbol::Data(index) => write!(f, "ra-jit.d.{index}"),
        }
    }
}

/// Why LLVM did not compile a function.
#[derive(Debug)]
pub(super) enum Unhandled {
    Opcode(Opcode),
    Type(Type),
    /// IR or an object of a kind there is no answer for yet.
    Shape(&'static str),
    /// A relocation type of the object format, by its number.
    Relocation(u32),
    Llvm(String),
    /// What an earlier build of the same IR was told, as the store kept it.
    Kept(String),
}

impl fmt::Display for Unhandled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unhandled::Opcode(opcode) => write!(f, "opcode `{opcode}`"),
            Unhandled::Type(ty) => write!(f, "type `{ty}`"),
            Unhandled::Shape(what) => f.write_str(what),
            Unhandled::Relocation(kind) => write!(f, "relocation of type {kind}"),
            Unhandled::Kept(reason) => f.write_str(reason),
            // The first line names the problem; the rest names the function's values.
            Unhandled::Llvm(message) => {
                write!(f, "llvm: {}", message.lines().next().unwrap_or_default())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use inkwell::{memory_buffer::MemoryBuffer, values::AnyValue};

    use super::*;

    /// Reduced with `llvm-reduce` from a function of bun 1.4.3 at `-O3` on AArch64, where one
    /// run of instruction combining after `default<O3>` leaves more to combine.
    const SHORT_OF_A_FIXPOINT: &str = r#"
define void @f(ptr %0, i1 %.not) personality ptr null {
frame:
  %1 = load volatile i32, ptr null, align 4
  %2 = icmp ne i32 %1, 0
  %.not7.not.not = select i1 %.not, i1 false, i1 %2
  br i1 %.not7.not.not, label %3, label %common.ret

3:
  %4 = load i32, ptr %0, align 1
  %5 = udiv i32 %1, %4
  %6 = icmp eq i32 %5, 1
  call void @llvm.assume(i1 %6)
  %7 = urem i32 %1, %4
  %8 = icmp ne i32 %7, 0
  call void @llvm.assume(i1 %8)
  br label %common.ret

common.ret:
  ret void
}

declare void @llvm.assume(i1 noundef)
"#;

    /// An export that calls itself and its request 1, and reads its datum 0.
    const EXPORTED: &str = r#"
@"ra-jit.d.0" = external global i32

define i32 @"ra-jit.own"(i32 %n) {
  %stop = icmp eq i32 %n, 0
  br i1 %stop, label %done, label %more
more:
  %less = sub i32 %n, 1
  %rest = call i32 @"ra-jit.f.0"(i32 %less)
  ret i32 %rest
done:
  %read = load i32, ptr @"ra-jit.d.0"
  %other = call i32 @"ra-jit.f.1"(i32 %read)
  ret i32 %other
}

declare i32 @"ra-jit.f.0"(i32)
declare i32 @"ra-jit.f.1"(i32)
"#;

    fn parsed<'ctx>(context: &'ctx Context, text: &str) -> Module<'ctx> {
        let text = format!("{text}\0");
        let source = MemoryBuffer::create_from_memory_range_copy(text.as_bytes(), "test");
        context.create_module_from_ir(source).unwrap()
    }

    #[test]
    fn an_import_is_of_the_compiles_requests_and_built_in() {
        let context = Context::create();
        let bitcode = parsed(&context, EXPORTED).write_bitcode_to_memory().as_slice().to_vec();
        let export = super::super::imports::Export {
            bitcode,
            requests: Default::default(),
            signature: ir::Signature::new(cranelift_codegen::isa::CallConv::SystemV),
            size: 8,
            hint: Hint::Inline,
        };
        // The export's request 0 is itself; its own 1 is the compile's 7, its datum 0 the 3.
        let import = Import {
            export: std::sync::Arc::new(export),
            id: 5,
            funcs: vec![5, 7],
            datas: vec![3],
        };
        // As `emit` has it: the function compiled is added, what it imports linked in, and
        // its body, which calls the import, lowered.
        let module = context.create_module("ra-jit");
        let i32 = context.i32_type();
        let own = module.add_function(OWN, i32.fn_type(&[], false), None);
        link(&context, &module, &import).unwrap();
        let body = module.get_function("ra-jit.f.5").expect("the import, linked in");
        assert_eq!(body.get_linkage(), Linkage::AvailableExternally);
        let builder = context.create_builder();
        builder.position_at_end(context.append_basic_block(own, ""));
        let called = builder.build_direct_call(body, &[i32.const_zero().into()], "").unwrap();
        let returned = called.try_as_basic_value().basic().unwrap();
        builder.build_return(Some(&returned)).unwrap();
        module.verify().unwrap();
        assert!(module.get_function("ra-jit.f.7").is_some_and(|it| it.count_basic_blocks() == 0));
        assert!(module.get_global("ra-jit.d.3").is_some());
        let names: Vec<String> =
            module.get_functions().map(|it| it.get_name().to_string_lossy().into_owned()).collect();
        assert!(
            !names
                .iter()
                .any(|it| it.contains(".f.0") || it.contains(".f.1") || it.contains("import")),
            "{names:?}"
        );
        let text = body.print_to_string().to_string_lossy().into_owned();
        assert!(text.contains("call i32 @ra-jit.f.5(i32"), "{text}");

        let optimization =
            Optimization { passes: Passes::Aggressive, selection: Selection::Optimized };
        let machine = Machine::create(optimization).unwrap();
        let pipeline = optimization.pipeline().unwrap();
        module.run_passes(pipeline, &machine, PassBuilderOptions::create()).unwrap();
        module.run_passes("elim-avail-extern", &machine, PassBuilderOptions::create()).unwrap();
        // Built in, the call is of the other function with the datum, and the body is gone.
        let own = module.get_function(OWN).unwrap();
        let text = own.print_to_string().to_string_lossy().into_owned();
        assert!(text.contains("@ra-jit.f.7(") && text.contains("@ra-jit.d.3"), "{text}");
        assert!(!text.contains("ra-jit.f.5"), "{text}");
        assert!(module.get_function("ra-jit.f.5").is_none_or(|it| it.count_basic_blocks() == 0));
    }

    #[test]
    fn instruction_combining_short_of_a_fixpoint_does_not_abort() {
        let optimization =
            Optimization { passes: Passes::Aggressive, selection: Selection::Optimized };
        let machine = Machine::create(optimization).unwrap();
        let context = Context::create();
        // LLVM reads to a nul.
        let text = format!("{SHORT_OF_A_FIXPOINT}\0");
        let source = MemoryBuffer::create_from_memory_range_copy(text.as_bytes(), "short");
        let module = context.create_module_from_ir(source).unwrap();
        let pipeline = optimization.pipeline().unwrap();
        module.run_passes(pipeline, &machine, PassBuilderOptions::create()).unwrap();
    }
}
