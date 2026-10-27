//! `_mm_fmadd_ps` and `_mm_fmadd_sd`, in a function with FMA, are LLVM's `vfmadd213ps` and
//! `vfmadd213sd` with the factors the other way round from libm's `fmaf` and `fma`, which
//! ra-jit called lane by lane: of two NaN factors rustc's kept the first and ra-jit's the
//! second. At `-O0` LLVM also folds the code around such a call into other forms
//! (`_mm_fnmadd_ss` is one `vfnmadd213ss`, `_mm_fmaddsub_ps` one `vfmaddsub213ps`), which keep
//! a NaN's sign where the negation in `core::arch` flips it, so ra-jit refuses a fused
//! multiply-add there and says so.

#[cfg(target_arch = "x86_64")]
pub fn run() -> [[u64; 4]; 4] {
    use std::arch::x86_64::*;
    if !is_x86_feature_detected!("fma") {
        return [[0; 4]; 4];
    }
    #[target_feature(enable = "fma")]
    fn fused(a: [u32; 4], b: [u32; 4], c: [u32; 4]) -> [[u64; 4]; 4] {
        unsafe {
            let [a, b, c]: [__m128; 3] = std::mem::transmute([a, b, c]);
            let singles = [_mm_fmadd_ps(a, b, c), _mm_fnmadd_ss(a, b, c), _mm_fmaddsub_ps(a, b, c)]
                .map(|it| std::mem::transmute::<_, [u32; 4]>(it).map(u64::from));
            let [x, y, z]: [__m128d; 3] = std::mem::transmute([
                [0x7ff8_0000_0000_0001_u64, 1],
                [0xfff8_0000_0000_0002, 2],
                [0x4010_0000_0000_0000, 3],
            ]);
            let doubles = std::mem::transmute::<_, [u64; 2]>(_mm_fmadd_sd(x, y, z));
            [singles[0], singles[1], singles[2], [doubles[0], doubles[1], 0, 0]]
        }
    }
    unsafe {
        fused(
            [0x7fc0_0001, 0x7f80_0011, 0x3fc0_0000, 0x7fc0_0001],
            [0xffc0_0002, 0xff80_0022, 0xffc0_0002, 0x4000_0000],
            [0x4080_0000, 0x7fc0_0003, 0x7f80_0033, 0x3f80_0000],
        )
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> [[u64; 4]; 4] {
    [[0; 4]; 4]
}
