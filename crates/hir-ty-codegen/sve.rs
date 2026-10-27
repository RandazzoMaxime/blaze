//! AArch64's scalable vectors (SVE and SVE2): `core::arch::aarch64`'s `svint32_t`, `svbool_t`, the
//! tuples of them and the internal `nxv2i8`-like types, whose size is a multiple of a vector
//! length only the machine knows.
//!
//! Cranelift has no type for them, so a scalable value lives in memory as its register image:
//! the bytes `str z` writes of a data vector (VL / 8 of them), those `str p` writes of a
//! predicate (VL / 64), the vectors of a tuple one after another, each in a place sized for the
//! largest vector length the architecture allows (2048 bits). Every place of a scalable type is
//! such storage of ra-jit's own (a local, the copy of an argument, a return slot), so a copy of
//! the whole of it is exact at any vector length. Between ra-jit's functions a scalable value is
//! passed and returned by reference. What reads one through a pointer, or passes one where
//! foreign code expects rustc's convention (in Z and P registers), is refused.
//!
//! Each operation is a call of an assembled function ([`stub`]) that loads its scalable operands
//! from the places it is handed (`ldr z`, `ldr p`), runs the instruction LLVM selects for the
//! intrinsic, and stores the result (`str z`, `str p`) or returns a scalar.

use std::fmt::Write;

use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature, Value};
use hir_def::FunctionId;
use hir_ty::{
    db::HirDatabase,
    layout::Layout,
    mir::{Operand, OperandKind, Rvalue, StatementKind},
    next_solver::{ConstKind, DbInterner, GenericArgs, Ty, TyKind},
    traits::StoredParamEnvAndCrate,
};
use rustc_abi::{BackendRepr, Size};
use rustc_type_ir::TypeVisitableExt;
use rustc_type_ir::inherent::{GenericArgs as _, IntoKind, SliceLike as _};
use triomphe::Arc;

use super::{CodegenError, FunctionCx, MEM, Result, ptr};

/// The bytes of a Z register at the largest vector length, 2048 bits.
pub(super) const DATA_MAX: u64 = 256;
/// The bytes of a P register then: one bit per byte of a Z register.
pub(super) const PREDICATE_MAX: u64 = DATA_MAX / 8;

/// What a scalable vector type holds, from its layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Shape {
    /// The bits of a lane: 1 for a predicate (`bool` lanes).
    pub(super) bits: u64,
    pub(super) float: bool,
    pub(super) signed: bool,
    /// Lanes per 128 bits of vector length.
    pub(super) count: u64,
    /// Vectors of a tuple, 1 for a vector alone.
    pub(super) vectors: u64,
}

impl Shape {
    pub(super) fn of(layout: &Layout) -> Option<Shape> {
        let BackendRepr::SimdScalableVector { element, count, number_of_vectors } =
            layout.backend_repr
        else {
            return None;
        };
        let (bits, float, signed) = if element.is_bool() {
            (1, false, false)
        } else {
            let primitive = element.primitive();
            let float = matches!(primitive, rustc_abi::Primitive::Float(_));
            let signed = matches!(primitive, rustc_abi::Primitive::Int(_, true));
            // A lane's size does not depend on the data layout's pointer width here: only
            // integers and floats are lanes of a scalable vector.
            let bits = match primitive {
                rustc_abi::Primitive::Int(int, _) => int.size().bits(),
                rustc_abi::Primitive::Float(float) => float.size().bits(),
                rustc_abi::Primitive::Pointer(_) => return None,
            };
            (bits, float, signed)
        };
        let vectors = u64::from(number_of_vectors.0);
        Some(Shape { bits, float, signed, count: count.as_u64(), vectors })
    }

    pub(super) fn predicate(self) -> bool {
        self.bits == 1
    }

    /// The bytes of the storage of one vector of it.
    pub(super) fn vector_storage(self) -> u64 {
        if self.predicate() { PREDICATE_MAX } else { DATA_MAX }
    }

    pub(super) fn storage(self) -> u64 {
        self.vector_storage() * self.vectors
    }

    /// The size of an element the register holds a lane in: its own for a data vector whose
    /// lanes fill the register (`nxv4i32`), wider for an unpacked one (`nxv2i8`'s lanes each
    /// in the low bits of 64), and for a predicate that of the data its lanes govern.
    pub(super) fn container_bits(self) -> u64 {
        128 / self.count
    }

    /// The instruction suffix of the elements the register holds.
    pub(super) fn suffix(self) -> Option<&'static str> {
        Some(match self.container_bits() {
            8 => "b",
            16 => "h",
            32 => "s",
            64 => "d",
            _ => return None,
        })
    }

    /// LLVM's name of the type, `nxv4i32`, `nxv2f64`, `nxv16i1`, as an intrinsic's name has it.
    pub(super) fn llvm(self) -> String {
        let kind = if self.float { 'f' } else { 'i' };
        format!("nxv{}{kind}{}", self.count, self.bits)
    }

    /// The letter of a load or store of one lane of memory: `ld1b` to `ld1d`.
    fn memory_size(self) -> Option<&'static str> {
        Some(match self.bits {
            8 => "b",
            16 => "h",
            32 => "w",
            64 => "d",
            _ => return None,
        })
    }

    /// Whether two types have one register image for the same value, so that rustc's
    /// `transmute` (LLVM's `bitcast`, which on a little-endian target reinterprets the register)
    /// is a copy of the image: two data vectors whose lanes fill the register, or two of one
    /// shape. An unpacked vector's `bitcast` to another shape moves lanes, which a copy of the
    /// register image does not, and a predicate is never bitcast to another kind.
    pub(super) fn same_image(self, other: Shape) -> bool {
        if self.vectors != other.vectors {
            return false;
        }
        let full = |it: Shape| !it.predicate() && it.bits * it.count == 128;
        (full(self) && full(other)) || (self.bits == other.bits && self.count == other.count)
    }
}

/// The layout codegen gives a scalable type: rustc's, sized for its storage at the largest
/// vector length rather than at the least.
pub(super) fn storage(layout: Arc<Layout>) -> Arc<Layout> {
    let Some(shape) = Shape::of(&layout) else { return layout };
    let mut storage = (*layout).clone();
    storage.size = Size::from_bytes(shape.storage());
    Arc::new(storage)
}

/// What an operand of a stub is.
#[derive(Clone, Copy, Debug)]
pub(super) enum Arg {
    /// A scalable vector, handed over as the address of its image.
    Vector(Shape),
    /// An integer, pointer or fieldless enum of so many bits, in a general register, and its
    /// value when it is a constant, which an instruction may take in its text instead.
    Int(u64, Option<i128>),
    /// A float of so many bits, in the low bits of a vector register.
    Float(u64),
}

/// What a stub gives.
#[derive(Clone, Copy, Debug)]
pub(super) enum Ret {
    /// Nothing.
    Unit,
    /// A scalable vector, written to the address handed over first.
    Vector(Shape),
    /// An integer of so many bits, in `x0`.
    Int(u64),
}

/// The registers a stub has its operands in, as AAPCS64 places the arguments of the
/// signature [`stub`] is called with: the address of the result in `x0` (as the first
/// argument), then each operand, an integer or address in the next general register, a float
/// in the next vector register. Each scalable operand is loaded into a Z or P register of its
/// own, after those the floats take.
struct Registers {
    /// The register each operand is in.
    operands: Vec<String>,
    /// The general register the result's address is in.
    result: Option<&'static str>,
    /// Loads of the scalable operands.
    loads: String,
}

impl Registers {
    fn new(args: &[Arg], ret: Ret) -> std::result::Result<Registers, String> {
        let mut x = 0u32;
        let result = match ret {
            Ret::Vector(_) => {
                x += 1;
                Some("x0")
            }
            _ => None,
        };
        let floats = args.iter().filter(|it| matches!(it, Arg::Float(_))).count() as u32;
        let (mut v, mut z, mut p) = (0u32, floats, 0u32);
        let mut operands = Vec::new();
        let mut loads = String::new();
        for arg in args {
            operands.push(match *arg {
                Arg::Int(bits, _) => {
                    let register = if bits == 64 { format!("x{x}") } else { format!("w{x}") };
                    x += 1;
                    register
                }
                Arg::Float(_) => {
                    v += 1;
                    format!("z{}", v - 1)
                }
                Arg::Vector(shape) => {
                    let address = format!("x{x}");
                    x += 1;
                    if shape.predicate() {
                        if shape.vectors != 1 {
                            return Err("a tuple of predicates".to_owned());
                        }
                        writeln!(loads, "ldr p{p}, [{address}]").unwrap();
                        p += 1;
                        format!("p{}", p - 1)
                    } else {
                        for at in 0..shape.vectors {
                            // The storage of each vector of a tuple is DATA_MAX bytes apart.
                            let base = if at == 0 {
                                address.clone()
                            } else {
                                writeln!(loads, "add x16, {address}, #{}", at * DATA_MAX).unwrap();
                                "x16".to_owned()
                            };
                            writeln!(loads, "ldr z{}, [{base}]", z + at as u32).unwrap();
                        }
                        z += shape.vectors as u32;
                        format!("z{}", z - shape.vectors as u32)
                    }
                }
            });
        }
        if x > 8 || v > 8 || z > 32 || p > 16 {
            return Err("more operands than registers".to_owned());
        }
        Ok(Registers { operands, result, loads })
    }
}

/// The text of a stub: `features` the assembler takes the instructions of (`sve`, `sve2`),
/// the loads, the body, and the store of the result from `z0` or `p0`.
fn text(features: &[&str], registers: &Registers, body: &str, ret: Ret) -> String {
    let mut text = String::new();
    for feature in features {
        writeln!(text, ".arch_extension {feature}").unwrap();
    }
    writeln!(text, ".globl ra_jit_asm_block\nra_jit_asm_block:").unwrap();
    text.push_str(&registers.loads);
    text.push_str(body);
    if let (Ret::Vector(shape), Some(address)) = (ret, registers.result) {
        for at in 0..shape.vectors {
            if shape.predicate() {
                writeln!(text, "str p{at}, [{address}]").unwrap();
            } else if at == 0 {
                writeln!(text, "str z0, [{address}]").unwrap();
            } else {
                writeln!(text, "add x16, {address}, #{}\nstr z{at}, [x16]", at * DATA_MAX).unwrap();
            }
        }
    }
    text.push_str("ret\n");
    text
}

/// The name AArch64's assembler gives an `svpattern`, a 5-bit immediate.
fn pattern(imm: i128) -> std::result::Result<String, String> {
    Ok(match imm {
        0 => "pow2".to_owned(),
        1..=8 => format!("vl{imm}"),
        9 => "vl16".to_owned(),
        10 => "vl32".to_owned(),
        11 => "vl64".to_owned(),
        12 => "vl128".to_owned(),
        13 => "vl256".to_owned(),
        29 => "mul4".to_owned(),
        30 => "mul3".to_owned(),
        31 => "all".to_owned(),
        14..=28 => format!("#{imm}"),
        _ => return Err(format!("pattern {imm}")),
    })
}

/// Whether `pg` is a predicate that governs the lanes of `data`, one vector of each.
fn lanes_of(pg: Shape, data: Shape) -> bool {
    pg.predicate() && pg.vectors == 1 && data.vectors == 1 && pg.count == data.count
}

/// The memory-size letter of a lane as wide as its container (`s` to `w`).
fn suffix_size(suffix: &str) -> &'static str {
    match suffix {
        "b" => "b",
        "h" => "h",
        "s" => "w",
        _ => "d",
    }
}

/// The suffix `b`, `h`, `s` or `d` of an LLVM intrinsic's predicate type, `nxv16i1` to `nxv2i1`.
fn predicate_suffix(ty: &str) -> Option<&'static str> {
    Some(match ty {
        "nxv16i1" => "b",
        "nxv8i1" => "h",
        "nxv4i1" => "s",
        "nxv2i1" => "d",
        _ => return None,
    })
}

/// The assembly of the stub that does what the LLVM intrinsic `name` (`aarch64.sve.cntd`,
/// without `llvm.`) does to operands `args`, giving `ret`. An error says what has no stub,
/// which the caller refuses with.
pub(super) fn stub(name: &str, args: &[Arg], ret: Ret) -> std::result::Result<String, String> {
    let unsupported = || Err(format!("LLVM intrinsic `{name}`"));
    let Some(rest) = name.strip_prefix("aarch64.sve.") else { return unsupported() };
    let parts: Vec<&str> = rest.split('.').collect();
    let registers = Registers::new(args, ret)?;
    let op = |index: usize| registers.operands[index].clone();
    let sve = &["sve"][..];
    let sve2 = &["sve", "sve2"][..];
    let body = match (&parts[..], args, ret) {
        // The elements of a vector, of a pattern of them: `cntb` to `cntd`. The count of a
        // whole vector of bytes is `rdvl x0, #1` in LLVM's code, the same number.
        (
            [count @ ("cntb" | "cnth" | "cntw" | "cntd")],
            [Arg::Int(32, Some(pattern))],
            Ret::Int(64),
        ) => {
            let pattern = self::pattern(*pattern)?;
            return Ok(text(sve, &registers, &format!("{count} x0, {pattern}\n"), ret));
        }
        // A predicate of the elements a pattern picks.
        (["ptrue", ty], [Arg::Int(32, Some(pattern))], Ret::Vector(_)) => {
            let Some(suffix) = predicate_suffix(ty) else { return unsupported() };
            let pattern = self::pattern(*pattern)?;
            format!("ptrue p0.{suffix}, {pattern}\n")
        }
        // The lanes of a predicate of wider elements as one of bytes: each lane's bit where
        // the bit of its first byte is, and the bits between them clear, which those of the
        // image of `nxv4i1` need not be.
        (["convert", "to", "svbool", ty], [Arg::Vector(_)], Ret::Vector(_)) => {
            let Some(suffix) = predicate_suffix(ty) else { return unsupported() };
            format!("ptrue p1.{suffix}\nand p0.b, p1/z, p0.b, p0.b\n")
        }
        // A predicate of bytes read as one of wider elements: an instruction on those reads
        // the bit of each element's first byte, and those between are left as they are.
        (["convert", "from", "svbool", ty], [Arg::Vector(_)], Ret::Vector(_)) => {
            if predicate_suffix(ty).is_none() {
                return unsupported();
            }
            String::new()
        }
        // The lanes up to the first that a write through one address and a read through the
        // other could overlap at.
        (
            [while_ @ ("whilerw" | "whilewr"), size @ ("b" | "h" | "s" | "d"), ty, "p0"],
            [Arg::Int(64, _), Arg::Int(64, _)],
            Ret::Vector(_),
        ) if predicate_suffix(ty) == Some(*size) => {
            let (a, b) = (op(0), op(1));
            return Ok(text(sve2, &registers, &format!("{while_} p0.{size}, {a}, {b}\n"), ret));
        }
        // The active lanes of a predicate that are also of the governing one.
        (["cntp", ty], [Arg::Vector(pg), Arg::Vector(op)], Ret::Int(64))
            if pg == op && pg.predicate() && pg.vectors == 1 && pg.llvm() == *ty =>
        {
            let suffix = pg.suffix().unwrap();
            format!("cntp x0, p0, p1.{suffix}\n")
        }
        // Each active lane read from memory, the others zero: of an unpacked vector, each in
        // the low bits of its element, which the load zero-extends into.
        ([load @ ("ld1" | "ldnt1"), ty], [Arg::Vector(pg), Arg::Int(64, _)], Ret::Vector(data))
            if lanes_of(*pg, data) && data.llvm() == *ty =>
        {
            let (Some(size), Some(suffix)) = (data.memory_size(), data.suffix()) else {
                return unsupported();
            };
            // LLVM has no non-temporal load of an unpacked vector.
            if *load == "ldnt1" && size != suffix_size(suffix) {
                return unsupported();
            }
            let base = op(1);
            format!("{load}{size} {{z0.{suffix}}}, p0/z, [{base}]\n")
        }
        // Each active lane written to memory, of an unpacked vector the low bits of each element.
        (
            [store @ ("st1" | "stnt1"), ty],
            [Arg::Vector(data), Arg::Vector(pg), Arg::Int(64, _)],
            Ret::Unit,
        ) if lanes_of(*pg, *data) && data.llvm() == *ty => {
            let (Some(size), Some(suffix)) = (data.memory_size(), data.suffix()) else {
                return unsupported();
            };
            if *store == "stnt1" && size != suffix_size(suffix) {
                return unsupported();
            }
            let base = op(2);
            format!("{store}{size} {{z0.{suffix}}}, p0, [{base}]\n")
        }
        _ => return unsupported(),
    };
    Ok(text(sve, &registers, &body, ret))
}

/// The stub of `simd_select` on scalable vectors: each lane of `on_true` where the mask's is
/// set, else of `on_false`. LLVM's `select` of them is SVE's `sel`.
pub(super) fn select(mask: Shape, value: Shape) -> std::result::Result<String, String> {
    if mask.vectors != 1 || value.vectors != 1 || mask.count != value.count || !mask.predicate() {
        return Err("`simd_select` of scalable vectors of other shapes".to_owned());
    }
    let ret = Ret::Vector(value);
    let args = [Arg::Vector(mask), Arg::Vector(value), Arg::Vector(value)];
    let registers = Registers::new(&args, ret)?;
    let body = if value.predicate() {
        "sel p0.b, p0, p1.b, p2.b\n".to_owned()
    } else {
        let Some(suffix) = value.suffix() else {
            return Err(format!("`simd_select` of {value:?}"));
        };
        format!("sel z0.{suffix}, p0, z0.{suffix}, z1.{suffix}\n")
    };
    Ok(text(&["sve"], &registers, &body, ret))
}

/// The stub of `simd_cast` between scalable vectors of integers with as many lanes: LLVM's
/// `sext` or `zext` (by the signedness of the source) is SVE's `sxt*`/`uxt*` of each lane in its
/// container, and its `trunc` leaves the register as it is, the bits above a lane's being
/// none of the narrower type's. `None` when the image is the same.
pub(super) fn cast(from: Shape, to: Shape) -> std::result::Result<Option<String>, String> {
    let ints = |it: Shape| !it.float && !it.predicate() && it.vectors == 1;
    if !ints(from) || !ints(to) || from.count != to.count {
        return Err(format!("`simd_cast` from {from:?} to {to:?}"));
    }
    if to.bits <= from.bits {
        return Ok(None);
    }
    let Some(suffix) = to.suffix() else { return Err(format!("`simd_cast` to {to:?}")) };
    let width = match from.bits {
        8 => "b",
        16 => "h",
        32 => "w",
        _ => return Err(format!("`simd_cast` from {from:?}")),
    };
    let extend = if from.signed { "sxt" } else { "uxt" };
    let ret = Ret::Vector(to);
    let registers = Registers::new(&[Arg::Vector(from)], ret)?;
    let body = format!("ptrue p0.{suffix}\n{extend}{width} z0.{suffix}, p0/m, z0.{suffix}\n");
    Ok(Some(text(&["sve"], &registers, &body, ret)))
}

/// Whether the signature of `func` with `args` (its own parameters when `None`) has a
/// scalable vector, which rustc's code passes in SVE registers.
pub(super) fn passes_scalable(
    db: &dyn HirDatabase,
    func: FunctionId,
    args: Option<GenericArgs<'_>>,
    env: &StoredParamEnvAndCrate,
) -> bool {
    let interner = DbInterner::new_no_crate(db);
    let sig = db.callable_item_signature(func.into());
    let sig = match args {
        Some(args) => sig.instantiate(interner, args),
        None => sig.instantiate_identity(),
    }
    .skip_binder();
    // A scalable vector type has no lifetimes nor parameters, so one with late-bound lifetimes
    // (`&'a T`) or parameters is none.
    let concrete = |ty: &Ty<'_>| !ty.has_escaping_bound_vars() && !ty.has_param();
    sig.inputs_and_output.iter().filter(concrete).any(|ty| {
        db.layout_of_ty(ty.store(), env.clone())
            .is_ok_and(|it| matches!(it.backend_repr, BackendRepr::SimdScalableVector { .. }))
    })
}

impl<'db> FunctionCx<'_, 'db> {
    /// The value of a constant operand of an integer or fieldless enum type, as the type's
    /// signedness extends it: `core::arch` gives an SVE intrinsic its immediates as such, as
    /// a const generic `svpattern`.
    fn sve_constant(&mut self, operand: &Operand, ty: Ty<'db>) -> Result<Option<i128>> {
        let layout = self.layout(ty)?;
        let BackendRepr::Scalar(scalar) = layout.backend_repr else { return Ok(None) };
        let size = layout.size;
        let signed = matches!(scalar.primitive(), rustc_abi::Primitive::Int(_, true));
        let bits = match ty.kind() {
            TyKind::Int(_) | TyKind::Uint(_) => self.immediate(operand)?,
            TyKind::Adt(..) => {
                let bytes = match &operand.kind {
                    OperandKind::Constant { konst, .. } => match konst.as_ref().kind() {
                        ConstKind::Value(value) => {
                            let blob = self.valtree_blob(ty, value.value)?;
                            (blob.relocs.is_empty() && blob.fn_relocs.is_empty())
                                .then_some(blob.bytes)
                        }
                        ConstKind::Unevaluated(named) => {
                            Some(self.eval_const(named)?.memory.to_vec())
                        }
                        _ => None,
                    },
                    OperandKind::Allocation { allocation } => {
                        Some(allocation.as_ref().memory.to_vec())
                    }
                    // A const generic argument is moved to a local of its own first.
                    OperandKind::Copy(place) | OperandKind::Move(place)
                        if place.projection.as_slice().is_empty() =>
                    {
                        let Some(body) = self.body else { return Ok(None) };
                        let mut assigned = body.basic_blocks.iter().flat_map(|(_, block)| {
                            block.statements.iter().filter_map(|it| match &it.kind {
                                StatementKind::Assign(to, rvalue) if to.local == place.local => {
                                    Some((to, rvalue))
                                }
                                _ => None,
                            })
                        });
                        let (Some((to, Rvalue::Use(from))), None) =
                            (assigned.next(), assigned.next())
                        else {
                            return Ok(None);
                        };
                        if to != place || self.operand_ty(from)? != ty {
                            return Ok(None);
                        }
                        let from = from.clone();
                        return self.sve_constant(&from, ty);
                    }
                    _ => None,
                };
                bytes.filter(|it| it.len() >= size.bytes_usize()).map(|it| {
                    let mut all = [0; 16];
                    all[..size.bytes_usize()].copy_from_slice(&it[..size.bytes_usize()]);
                    u128::from_le_bytes(all)
                })
            }
            _ => None,
        };
        Ok(bits.map(|bits| {
            let shift = 128 - size.bits();
            if signed { ((bits << shift) as i128) >> shift } else { bits as i128 }
        }))
    }

    /// A call of the LLVM intrinsic `name` (`aarch64.sve.*`) as a call of its stub.
    pub(super) fn sve_intrinsic(
        &mut self,
        name: &str,
        args: &[Operand],
        dst: Value,
        dst_ty: Ty<'db>,
    ) -> Result<()> {
        let mut kinds = Vec::new();
        let mut values = Vec::new();
        let mut params = Vec::new();
        for arg in args {
            let ty = self.operand_ty(arg)?;
            if let Some(shape) = self.scalable(ty)? {
                let (addr, _) = self.operand(arg)?;
                kinds.push(Arg::Vector(shape));
                values.push(addr);
                params.push(AbiParam::new(ptr()));
                continue;
            }
            let layout = self.layout(ty)?;
            let BackendRepr::Scalar(_) = layout.backend_repr else {
                unsupported!("LLVM intrinsic `{name}` with an operand of {ty:?}");
            };
            let constant = self.sve_constant(arg, ty)?;
            let (value, scalar) = self.load_scalar(arg)?;
            let bits = u64::from(scalar.ty.bits());
            kinds.push(if scalar.ty.is_float() {
                Arg::Float(bits)
            } else {
                Arg::Int(bits, constant)
            });
            values.push(value);
            params.push(AbiParam::new(scalar.ty));
        }
        let mut sig = self.cg.make_signature();
        let ret = if let Some(shape) = self.scalable(dst_ty)? {
            sig.params.push(AbiParam::new(ptr()));
            values.insert(0, dst);
            Ret::Vector(shape)
        } else if self.layout(dst_ty)?.is_zst() {
            Ret::Unit
        } else {
            let scalar = self.scalar_of(dst_ty)?;
            if scalar.ty.is_float() {
                unsupported!("LLVM intrinsic `{name}` giving {dst_ty:?}");
            }
            sig.returns.push(AbiParam::new(scalar.ty));
            Ret::Int(u64::from(scalar.ty.bits()))
        };
        sig.params.extend(params);
        let text = stub(name, &kinds, ret).map_err(CodegenError::Unsupported)?;
        self.call_stub(text, &sig, &values, ret, dst);
        Ok(())
    }

    fn call_stub(&mut self, text: String, sig: &Signature, values: &[Value], ret: Ret, dst: Value) {
        let callee = self.cg.assembly(text, sig);
        let callee = self.cg.declare_func_in_func(callee, self.builder.func);
        let call = self.builder.ins().call(callee, values);
        if let Ret::Int(_) = ret {
            let result = self.builder.inst_results(call)[0];
            self.builder.ins().store(MEM, result, dst, 0);
        }
    }

    /// The Rust intrinsics `core::arch`'s SVE functions use on scalable vectors. `false` when
    /// `name` is none of them or has no scalable vector here.
    pub(super) fn scalable_intrinsic(
        &mut self,
        name: &str,
        generic_args: GenericArgs<'db>,
        args: &[Operand],
        dst: Value,
        dst_ty: Ty<'db>,
    ) -> Result<bool> {
        let ours = name.starts_with("sve_tuple_") || matches!(name, "simd_select" | "simd_cast");
        let Some(to) = self.scalable(dst_ty)?.filter(|_| ours) else { return Ok(false) };
        let mut operands = Vec::new();
        for arg in args {
            let (addr, ty) = self.operand(arg)?;
            operands.push((addr, self.scalable(ty)?));
        }
        let vector = Size::from_bytes(DATA_MAX);
        match (name, &operands[..]) {
            ("sve_tuple_create2" | "sve_tuple_create3" | "sve_tuple_create4", _) => {
                if operands.len() as u64 != to.vectors
                    || operands
                        .iter()
                        .any(|(_, it)| it.is_none_or(|it| it.vectors != 1 || it.predicate()))
                {
                    unsupported!("`{name}` of {dst_ty:?}");
                }
                for (at, &(src, _)) in operands.iter().enumerate() {
                    let place = self.builder.ins().iadd_imm(dst, at as i64 * DATA_MAX as i64);
                    self.copy(place, src, vector);
                }
            }
            ("sve_tuple_get" | "sve_tuple_set", _) => {
                let Some(index) = generic_args.iter().nth(2).and_then(|it| it.konst()) else {
                    unsupported!("`{name}` without its index");
                };
                let ConstKind::Value(index) = index.kind() else {
                    unsupported!("`{name}` with an index not evaluated");
                };
                let index = index.value.inner().to_leaf().to_bits(Size::from_bytes(4)) as i32;
                match (name, &operands[..]) {
                    ("sve_tuple_get", [(tuple, Some(of))])
                        if (0..of.vectors as i32).contains(&index) && to.vectors == 1 =>
                    {
                        let place =
                            self.builder.ins().iadd_imm(*tuple, index as i64 * DATA_MAX as i64);
                        self.copy(dst, place, vector);
                    }
                    ("sve_tuple_set", [(tuple, Some(of)), (value, Some(one))])
                        if (0..of.vectors as i32).contains(&index)
                            && one.vectors == 1
                            && *of == to =>
                    {
                        self.copy(dst, *tuple, Size::from_bytes(to.storage()));
                        let place =
                            self.builder.ins().iadd_imm(dst, index as i64 * DATA_MAX as i64);
                        self.copy(place, *value, vector);
                    }
                    _ => unsupported!("`{name}` of {dst_ty:?} at {index}"),
                }
            }
            ("simd_select", [(mask, Some(mask_shape)), (a, Some(value)), (b, Some(_))]) => {
                let text = select(*mask_shape, *value).map_err(CodegenError::Unsupported)?;
                let mut sig = self.cg.make_signature();
                sig.params.extend([AbiParam::new(ptr()); 4]);
                self.call_stub(text, &sig, &[dst, *mask, *a, *b], Ret::Vector(to), dst);
            }
            ("simd_cast", [(src, Some(from))]) => {
                match cast(*from, to).map_err(CodegenError::Unsupported)? {
                    None => self.copy(dst, *src, Size::from_bytes(to.storage())),
                    Some(text) => {
                        let mut sig = self.cg.make_signature();
                        sig.params.extend([AbiParam::new(ptr()); 2]);
                        self.call_stub(text, &sig, &[dst, *src], Ret::Vector(to), dst);
                    }
                }
            }
            _ => unsupported!("intrinsic `{name}` on scalable vectors"),
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S32: Shape = Shape { bits: 32, float: false, signed: true, count: 4, vectors: 1 };
    const B4: Shape = Shape { bits: 1, float: false, signed: false, count: 4, vectors: 1 };
    const B16: Shape = Shape { bits: 1, float: false, signed: false, count: 16, vectors: 1 };

    #[test]
    fn counts_take_their_pattern() {
        let text = stub("aarch64.sve.cntd", &[Arg::Int(32, Some(31))], Ret::Int(64)).unwrap();
        assert!(text.contains("cntd x0, all\nret"), "{text}");
        let text = stub("aarch64.sve.cntb", &[Arg::Int(32, Some(9))], Ret::Int(64)).unwrap();
        assert!(text.contains("cntb x0, vl16\n"), "{text}");
    }

    #[test]
    fn predicate_conversion_clears_the_bits_between_lanes() {
        let text =
            stub("aarch64.sve.convert.to.svbool.nxv4i1", &[Arg::Vector(B4)], Ret::Vector(B16))
                .unwrap();
        assert!(text.contains("ldr p0, [x1]\nptrue p1.s\nand p0.b"), "{text}");
        assert!(text.contains("str p0, [x0]\nret"), "{text}");
    }

    #[test]
    fn whilerw_takes_two_addresses() {
        let args = [Arg::Int(64, None), Arg::Int(64, None)];
        let text = stub("aarch64.sve.whilerw.s.nxv4i1.p0", &args, Ret::Vector(B4)).unwrap();
        assert!(text.contains(".arch_extension sve2\n"), "{text}");
        assert!(text.contains("whilerw p0.s, x1, x2\n"), "{text}");
        assert!(stub("aarch64.sve.whilerw.d.nxv4i1.p0", &args, Ret::Vector(B4)).is_err());
    }

    #[test]
    fn loads_and_stores_of_unpacked_vectors_take_the_lane_size() {
        let bytes_in_words = Shape { bits: 8, float: false, signed: true, count: 4, vectors: 1 };
        let ptr = Arg::Int(64, None);
        let text =
            stub("aarch64.sve.ld1.nxv4i8", &[Arg::Vector(B4), ptr], Ret::Vector(bytes_in_words))
                .unwrap();
        assert!(text.contains("ldr p0, [x1]\nld1b {z0.s}, p0/z, [x2]\nstr z0, [x0]\n"), "{text}");
        let args = [Arg::Vector(S32), Arg::Vector(B4), ptr];
        let text = stub("aarch64.sve.st1.nxv4i32", &args, Ret::Unit).unwrap();
        assert!(text.contains("ldr z0, [x0]\nldr p0, [x1]\nst1w {z0.s}, p0, [x2]\nret"), "{text}");
        assert!(stub("aarch64.sve.st1.nxv4i8", &args, Ret::Unit).is_err());
    }

    #[test]
    fn select_of_words() {
        let text = select(B4, S32).unwrap();
        assert!(text.contains("ldr z0, [x2]\nldr z1, [x3]\nsel z0.s, p0, z0.s, z1.s\n"), "{text}");
    }

    #[test]
    fn images_of_full_vectors_are_one() {
        let f64s = Shape { bits: 64, float: true, signed: true, count: 2, vectors: 1 };
        let i8s_unpacked = Shape { bits: 8, float: false, signed: true, count: 2, vectors: 1 };
        let i16s_unpacked = Shape { bits: 16, float: false, signed: true, count: 2, vectors: 1 };
        assert!(S32.same_image(f64s));
        assert!(!i8s_unpacked.same_image(i16s_unpacked));
        assert!(!B4.same_image(B16));
    }
}
