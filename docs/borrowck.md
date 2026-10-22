# The borrow checker

ra-jit has its own borrow checker, the crate `hir-ty-borrowck` in the rust-analyzer subtree
(`vendor/rust-analyzer/crates/hir-ty-borrowck`, next to `hir-ty`). It checks the MIR rust-analyzer
lowers, the same MIR the code generator compiles, and it is meant to give exactly rustc's
verdict: the same errors, on the same lines, and none where rustc gives none. It gives
rustc's verdict on 85% of rustc's own borrow-check tests today, and on every one rustc
accepts; how they are scored and what is left is below.

It runs as part of rust-analyzer's diagnostics, so the daemon computes it for every edited
file, and `ra-jit borrowck` prints what it finds. The daemon refuses an edit it finds a
borrow-check error in (below), but the checker still misses about a sixth of the errors
rustc gives, so `cargo check` stays the final word.

## Refusing builds

An edit with a borrow-check error fails its build, with the file, line and error, as an edit
with a type error does; under `cargo ra-jit`, cargo then runs and rustc reports it.
`RA_JIT_BORROWCK_REFUSE=0` in the daemon's environment turns this off.

This is safe because the checker reports nothing on any test rustc accepts, nor on ripgrep,
fd, bat, hyperfine, just, axum or nushell: a refused build is one rustc refuses too. A
borrow-check error ra-jit reports where rustc reports none is a bug; `RA_JIT_BORROWCK_REFUSE=0`
works around it until it is fixed. What it does not do: it misses about a sixth of the
errors rustc gives (it accepts code rustc rejects, listed below), so code with such an error
still builds and runs from memory.

## Modes

`RA_JIT_BORROWCK` picks one:

- `polonius`, the default: rustc's Polonius alpha, which is what nightly rustc does by
  default (`-Zpolonius=next`). It accepts more programs than NLL: a loan ends where nothing
  that can still reach it is live, which accepts the "conditional return of a borrow" cases
  NLL rejects.
- `nll`: stable rustc's checker. The only difference from `polonius` is when a loan goes out
  of scope.
- `off`.

## What rustc rules it follows

All of these are rustc's, written from `compiler/rustc_borrowck` at the pinned nightly
([research/borrowck.md](research/borrowck.md) has the sources):

- Moves and initialization: moves out of places, partial moves, maybe-uninitialized reads,
  assignment to immutable locals (E0381, E0382, E0384, E0505, E0507, E0508, E0509). A `match`
  whose arms test the value reads what it matches first, so a value moved out whole is
  reported there. A `match` arm's guard sees the variables its pattern binds by reference, so
  it cannot move out of them (E0507), and casting an enum to an integer moves the enum
  (`*e as usize` behind a reference is E0507).
- Unaligned references (E0793): a `&` or `&mut` of a field a `#[repr(packed)]` struct may
  leave less aligned than its type needs, as rustc's `check_packed_ref` (not `&raw`, nor
  behind a pointer; a field of a type parameter or of a `dyn` tail may be unaligned).
- Conflicting borrows: shared, mutable, two-phase (`vec.push(vec.len())`), shallow borrows of
  match scrutinees, and fake reads (E0499, E0500, E0501, E0502, E0503, E0506, E0524). The drop
  of the old value before an assignment is the assignment's write, so assigning to a borrowed
  place is E0506 rather than E0597. A `match` reads a discriminant, and switches on a `bool`,
  integer or `char` it tests against a literal, at what it matches rather than at the arm.
- Errors for a borrow that does not live long enough, as rustc chooses among them: E0713 when
  the borrow is of a field of a value whose type implements `Drop` and the drop is of that
  value, E0373 ("may outlive the current block") for a closure a `let` binds as a block's
  value, and E0597 rather than E0515 for what a function returns as an `impl Trait` that
  captures no lifetime (rustc's `OpaqueType` constraint). The mutable borrows of a variable
  declared without `mut` are one E0596, at the declaration once there are two.
- `'static` in what a type or signature spells (`x: &'static u32` in a struct, a parameter
  `&'static T`) is related as any other region; a copied value proves its type `Copy`
  (`impl Copy for Foo<'static>`); a loop's way back may unwind as rustc's `FalseUnwind`, so a
  borrow that must outlive the function is reported in a loop that never ends; and
  `dyn Any + 'r` in the signature implies `'r: 'static`, which `trait Any: 'static` requires.
  The type an object binds an associated type to is invariant (`dyn Trait<Out = &'a u32>`),
  and unsizing to `dyn Trait` proves the value's type implements `Trait`, with what its impl
  needs of the regions.
- Well-formedness in bodies: a called function's signature as written, before its associated
  types are normalized, and its generic arguments are well-formed at the call (`&'a &'b ()`
  that `'b: 'a`); so is a function item's signature where it becomes a function pointer, with
  its late-bound lifetimes instantiated. A function item is well-formed with its where clauses
  and return type, and an associated type `<T as Trait>::Ty` where `T` implements `Trait`
  (`impl Trait for &'static ()` makes `<&'a () as Trait>::Ty` need `'a: 'static`). The
  generic arguments a path spells out are well-formed, and a called function's where clauses
  hold for them, in code that is never reached too.
- Promotion: a borrowed function item, and a named `const` whose type has no interior
  mutability whatever its drop (rustc asks whether the value needs dropping, not the type).
  In a `const` or `static` item's value, a call of a `const fn` is promotable too
  (`Migrations::new(&[Migration::new("a")])`), as it is in rustc's const contexts.
- Region inference: liveness of locals (drops kept live only back to where the local may be
  initialized), outlives constraints from every statement, call, cast and aggregate, the
  signature's universal regions and implied bounds (normalized parameters and the impl
  header included, for associated consts too), where clauses (elaborated through
  supertraits) and the region constraints of proving them with the trait solver (a where
  clause or impl that proves a bound relates its regions to the bound's, `&'max G: Get` proving
  `&'min G: Get` makes them equal, and the impl's own `T: 'a` must hold), type tests, and lifetimes written in `let` annotations (`let _: T = value`
  of any value included, and of each variable a destructuring pattern binds, `y` in
  `let (y, _): (&'static u32, u32)`), struct and variant patterns that spell out generic
  arguments (`let Foo::<'static> { .. } = foo`) and constant patterns (`<() as Foo<'static>>::C`,
  compared as `PartialEq` compares), casts, turbofishes (a method's own, `x.f::<&'a T>()`, too) and struct
  literal and variant paths, in closures too; `Self { .. }` and `Self(..)` in an impl have the
  impl's self type. A function item called through a variable
  (`let f = foo::<&'static u32>; f(&x)`, or through a reference, `let f = &foo::<..>`) has the
  variable's type, and the variable is used where it is called (a captured constructor too).
  A closure called through a variable takes its arguments as its signature says, whose
  lifetimes rustc infers once, in its type (`let f = |_| {}; f(&x)` in a loop needs `x` to
  live as long as `f`), unless it writes a parameter's or its return's type. `self` of a
  method of `impl dyn Trait` is `dyn Trait + 'static`, as the impl's header defaults it. A struct literal proves the struct's where clauses, an array
  repeat `[x; N]` of more than one copy proves `x`'s type `Copy`, and a path that names an
  associated item through a type (`Foo::f`, `<()>::f`, `T::f`, not `Default::default`) has its
  generic arguments and its impl's self type and trait arguments well-formed, as rustc's user
  type annotation on it. A function item, closure or safe function pointer cast to a
  function pointer is a subtype of it by signature. A where clause `for<'a> T: 'a` is
  `T: 'static`, and a type parameter that fails it is rustc's codeless "does not live long
  enough".
- Errors on universal regions: "lifetime may not live long enough", E0521, E0309, E0310,
  E0311, with rustc's object lifetime defaults (`&'a dyn Trait` is `dyn Trait + 'a`, a trait
  bounded by `'static` makes its objects `'static`, elsewhere in a body the lifetime is
  inferred).
- Loans that outlive their storage: E0597, E0515, E0716, and promoted constants (calls of
  `#[rustc_promotable]` functions such as `size_of` included) and extended temporaries of
  `const` and `static` items as `'static`.
- `impl Trait` captures: a return-position `impl Trait` keeps alive and relates only the
  lifetimes it captures, those listed in `use<..>` or, before edition 2024 outside traits and
  `async fn`, those its bounds name; its type parameters are always captured.
- `impl Trait`'s hidden type: each of its regions must be `'static` or one of the lifetimes
  the `impl Trait` captures (rustc's member constraints), else E0700 at the signature; and it
  must outlive what the bounds say (`impl Copy + 'a`). rustc picks the captured lifetime a
  region becomes and checks the body with that choice; ra-jit only tests that one exists, so
  it reports E0700 where rustc does and leaves the errors that choice causes unreported.
- Closures: captures, E0373, `FnMut` escapes, E0596 and E0594 through captures. A closure's
  body has two kinds of universal region, as rustc's: its own (the late-bound lifetimes of
  its signature, `for<'a> FnOnce(&'a T)`, and the borrow of its environment) and its
  creator's (every other region of its type, and the enclosing item's lifetime parameters,
  late-bound ones included, so `foo::<&'a u32>(&local)` in a closure needs `local` to outlive
  the creator's `'a`).
  One of its own made to outlive what it is not known to is an error in the closure
  (`with(|x| x)` returning the borrow it was given), named E0521 or a `FnMut` escape as
  rustc names it. What a closure needs of its creator's regions is a requirement the creator
  proves where it makes the closure (closures are checked before the bodies that make them),
  so the error is the creator's, as in rustc. So is a `T: 'a` the closure cannot prove: the
  creator proves `T` outlives each of its regions that outlive `'a`, and an error is
  reported where the closure needed it.
- `async` blocks and `async fn` bodies: checked as rustc checks a coroutine, as a closure
  of the body that makes it, for moves, initialization and loans across `.await`. Every
  region of the future's type is its creator's, and an `async fn` relates the future's
  output to its declared return type with lifetime elision applied (the lifetime of the
  one `&Self` in `self: Box<&Self>` or `Pin<&mut Self>` too), as rustc proves the
  `Future<Output = ..>` bound of the `impl Future` it returns.
- Items, as rustc's `wfcheck` (the crate `hir-ty-wfcheck`, next to the checker): a struct's,
  enum's or union's fields must be well-formed given its where clauses and what rustc infers
  from the fields (every `T: 'a`, never `T: 'static`), so `&'static T` or `Bar<T>` with
  `struct Bar<T: 'static>` in a field needs `T: 'static` written (E0310). A trait impl's
  associated types must be well-formed given the impl's where clauses and what its header's
  types imply (E0309), and the impl must satisfy the trait's outlives bounds, as
  `trait Foo: 'static` (E0310); a generic associated type is checked with its own where
  clauses, not the trait's. The where clauses of an ADT, impl, trait or function must be
  well-formed given its bounds and what its signature's and impl header's types imply
  (`fn foo<'a, L: X<&'a Nested<K>>>()` needs `K: 'a`). As rustc, an item that also needs a lifetime to outlive one
  it does not (E0491) gets only that error, which ra-jit does not report yet.
- E0621 ("explicit lifetime required") where rustc gives it: not in a trait impl's method,
  nor for an anonymous lifetime of `self` or of the return type.
- Where errors are: at the expression rustc points to. A borrow `format_args!` (so
  `println!`, `format!`, `write!`) makes of an argument has no syntax of its own, so its
  errors are at the argument, or for a variable the template names (`"{x}"`) at the
  template.

## How it is tested

`harness/borrowck_ui.py <rust checkout>` scores it against rustc's own UI tests at the
pinned nightly's commit: everything under `tests/ui/borrowck` and `tests/ui/nll`, every test
with a Polonius revision, and every other UI test whose expected output has a borrow-check
error. Each revision of a test is a unit. What a unit should give is what the pinned nightly
rustc gives on it, once with Polonius and once without; those runs are checked against the
tests' own `//~ ERROR` annotations so that a harness mistake shows.

A unit passes when ra-jit's set of borrow-check errors, as (error code, line), is rustc's
set. Errors rustc gives no code are named by their message. A unit is out of scope, with a
counted reason, when it cannot say anything about the borrow checker: it needs a nightly
feature, an auxiliary crate or another file, a `-Z` flag other than Polonius, or rustc
stops before borrow checking.

With no borrow checker, the 245 units rustc accepts would pass. Now:

| Mode | In scope | Pass | rustc accepts, ra-jit too | rustc rejects, same errors |
|---|---|---|---|---|
| Polonius | 1579 | 1337 (84.7%) | 245 of 245 | 1092 of 1334 |
| NLL | 1577 | 1329 (84.3%) | 226 of 226 | 1103 of 1351 |

463 units are out of scope; 344 of them need a nightly feature. CI runs the scoring on Linux
and fails below a floor a few units under the count there.

The other check is that it finds nothing in code rustc accepts.
`ra-jit borrowck --project DIR` checks a cargo workspace; on ripgrep, fd, bat, hyperfine,
just, axum, nushell, delta, eza, tokei and starship it finds nothing, nor on the harness's
`corpus` and `mixfutures` crates.

## What is left

Every failing unit is one where rustc gives an error ra-jit does not, or gives it on another
line or with another code. By what is wrong, most first: 29 "lifetime may not live long
enough" not reported, 21 E0597 not reported, 16 "lifetime may not live long enough" on
another line, 12 E0309, 11 E0310 and 10 E0521 not reported, then a long tail. The causes, grouped:

- **Higher-ranked regions and placeholders** (`for<'a>` bounds, closures with higher-ranked
  signatures): rustc checks them with placeholders in universes; ra-jit does not yet, and
  leaves out what a `for<'a>` lifetime is related to, apart from `for<'a> T: 'a`.
- **Well-formedness of items** (most of what is left of E0309 and E0310): what proving a
  trait bound needs of the regions (`<T as Dummy<'a>>::Out` in a field, proven by an impl
  that needs `T: 'a`), a projection's `<T as Trait>::Assoc: 'a`, and items whose signature
  or impl header has an associated type (what normalizing it implies) are not checked yet.
- **Written closure parameter types**: `|x: &'x u32|` gives `x` the creator's `'x` in rustc,
  the closure's own lifetime in ra-jit, so an error the annotation causes is reported where
  `x` is used instead of at the parameter.
- **Member constraints of `impl Trait`**: the choice rustc makes is not applied, and a
  projection bound that names a lifetime (`impl Iterator<Item = Box<dyn Foo>>`) leaves them
  out.
- **Match fake edges**: rustc's MIR gives each arm, guard and or-pattern alternative an
  imaginary edge from the one before, so borrowck cannot tell an arm is unreachable or a
  guard always runs. rust-analyzer lowers arms one after the other and none after an
  irrefutable one, so a move in an arm that never runs is not seen (`nll/match-cfg-fake-edges.rs`).
- **Captures of an empty `match`**: rustc's capture analysis has a closure that matches
  on `x` with no arms read `x`; rust-analyzer's captures nothing there, so the E0381 of an
  uninitialized `x` is missed.
- **Trait selection's region obligations**: an impl's `T: 'static` proven at an unsizing
  cast. A bound on a closure's type is not proven through the solver: rust-analyzer gives a
  closure one signature, `fn(&'1 T)` for one `'1`, where rustc's is `for<'a> fn(&'a T)`, so
  proving it would tie every call's borrow to one region.
- **Differences in rust-analyzer's MIR lowering** (below).

## What depends on rust-analyzer's MIR lowering

The borrow checker reads `hir-ty`'s MIR, so some verdicts depend on how that lowers. These
changes to rust-analyzer were needed, each made as rustc does it:

- Two-phase borrows for autoref method receivers (`vec.push(vec.len())`).
- `if let ... && ...` chains branch directly instead of through a boolean temporary.
- A match guard reads what its arm binds by value through a reference, and the arm moves
  it in only once the guard holds. A guard with `let` (`if let` guards and chains) branches
  on each operand as `if` does, so what its `let` binds is initialized where it holds. Its
  temporaries stay the arm's in every edition, as rustc keeps them (dropped when the arm ends
  or before the next arm is tried), where an `if let` before 2024 keeps them to its statement.
- Slice pattern indices carry the pattern's minimum length (`ConstantIndex::min_length`).
- Promotion: a division by a constant other than 0 (and -1 for a signed dividend that may be
  the minimum), and `&mut []`.
- Coercion (`hir-ty/src/infer/coerce.rs`): `&T` to `&T` reborrows. rust-analyzer gives elided
  lifetimes in signatures an error region, so two unrelated ones compared equal and the
  reborrow was skipped as a no-op; a closure then captured the reference variable rather
  than what it points to.
- A `let`'s fake read of its value is at the pattern (rustc's `FakeReadCause::ForLet`), not
  the enclosing block, which is where the borrow checker finds the place `let _: T` ascribes.
- A function item or a tuple struct's or variant's constructor written as a value
  (`let f = foo::<T>;`) is assigned to where it goes, though zero sized, so a closure that
  captures `f` or a method that takes it by value does not read it uninitialized (E0381).
- The trait solver (`hir-ty/src/next_solver/solver.rs`) answers with the region constraints
  of what it proves, as rustc's does, for an interner made with `with_regions()`: the
  borrow checker's. rust-analyzer's dropped them; type inference does not ask for them and
  its answers are cached apart.

These are differences still open, each the cause of some failures, all of them errors
missed rather than errors made up:

- Temporaries of a block's tail expression in edition 2021 are dropped at a different point.
- Comparisons against constant patterns take their operands by value.
- Unwind paths are not lowered. rustc's MIR gives every call and drop a cleanup path ending
  at `resume`, where any borrow still live that outlives the function's own regions is an
  error (E0597 for `let x = ...; let r: &'a _ = &x; f();`). ra-jit checks that at each call,
  drop and assert as if the path were there; it would be exact if the lowering added them.

## What it costs

On every edit the daemon runs the diagnostics of the edited files on a thread of its own,
beside the build, so the check adds to an edit only when it takes longer than the build.
For an edit of the corpus's largest file, the check went from a median of 40 ms with the
checker off to 52-74 ms with it on, while the build took 250-400 ms. A cold check of all of
nushell used 10-25% more CPU time. Both were measured on a loaded machine, so read them as
rough.

## Debugging

- `echo file.rs | ra-jit borrowck --edition 2021` prints a file's diagnostics as JSON.
- `RA_JIT_BORROWCK_MIR=<function>` with that command prints the MIR of a top-level function.
- `RA_JIT_BORROWCK_DEBUG=1` prints each body's locals, universal regions, constraints and
  loans.
