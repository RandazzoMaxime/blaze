/// Left to rustc: the daemon does not compile a constant of `Location::caller()`, so `main`,
/// rustc's, calls rustc's copy of this, and an edit of `shared` has cargo build the program.
#[inline(never)]
fn left() -> u64 {
    const CALLER: &std::panic::Location<'static> = std::panic::Location::caller();
    std::hint::black_box(CALLER.line());
    shared::answer()
}

fn main() {
    println!("stale {}", left());
}
