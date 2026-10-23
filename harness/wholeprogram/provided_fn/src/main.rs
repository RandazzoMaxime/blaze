// rustc compiles a function declared in a trait's provided method only where it instantiates
// the method, so its build has no symbol of `double`: ra-jit names it, and a call that is not
// built in is of that name (bstr's `ByteVec::from_os_str_lossy::imp`, in ripgrep).
trait Widen {
    fn widened(&self) -> u64;

    fn twice(&self) -> u64 {
        #[inline(never)]
        fn double(it: u64) -> u64 {
            std::hint::black_box(it) * 2
        }
        double(self.widened())
    }
}

impl Widen for u32 {
    fn widened(&self) -> u64 {
        u64::from(*self)
    }
}

impl Widen for u8 {
    fn widened(&self) -> u64 {
        u64::from(*self) + 1
    }
}

fn main() {
    let n = std::env::args().count();
    println!("{} {}", (n as u32).twice(), (n as u8).twice());
}
