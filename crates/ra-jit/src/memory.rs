//! Where the daemon's memory is.
//!
//! With `RA_JIT_MEMORY` naming a directory, the daemon reports what the process holds after
//! each step of warming up, and once warm and after each request it answers writes there what
//! every ingredient of the database holds. A build with the `heap-profile` feature allocates with jemalloc instead of
//! mimalloc: it can tell how much of the heap is live, and with `_RJEM_MALLOC_CONF=prof:true`
//! it writes a profile of the heap next to the ingredients, for `jeprof` to read.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

use ide_db::{RootDatabase, base_db::salsa};

/// The directory `RA_JIT_MEMORY` names, when it is set.
pub(crate) fn report_dir() -> Option<PathBuf> {
    std::env::var_os("RA_JIT_MEMORY").map(PathBuf::from)
}

/// Megabytes of memory the process has resident.
#[cfg(not(target_os = "macos"))]
pub(crate) fn resident() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find_map(|it| it.strip_prefix("VmRSS:"))?;
    Some(line.trim().trim_end_matches("kB").trim().parse::<u64>().ok()? / 1024)
}

/// Megabytes of memory the process owns: its physical footprint, which is what Activity
/// Monitor shows, and which still counts pages macOS has compressed.
#[cfg(target_os = "macos")]
pub(crate) fn resident() -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    // SAFETY: `info` has room for the `RUSAGE_INFO_V4` record the call writes.
    let pid = std::process::id() as libc::c_int;
    let ok = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, info.as_mut_ptr().cast()) };
    // SAFETY: the call wrote the record when it returned 0, and zeroes are a record too.
    (ok == 0).then(|| unsafe { info.assume_init() }.ri_phys_footprint >> 20)
}

/// Bytes allocated and not freed, when the allocator counts them.
pub(crate) fn live() -> Option<u64> {
    #[cfg(feature = "heap-profile")]
    {
        tikv_jemalloc_ctl::epoch::advance().ok()?;
        tikv_jemalloc_ctl::stats::allocated::read().ok().map(|it| it as u64)
    }
    #[cfg(not(feature = "heap-profile"))]
    None
}

/// Reports what the process holds now that `step` is done.
pub(crate) fn checkpoint(step: &str) {
    if report_dir().is_none() {
        return;
    }
    let live = live().map(|it| format!(", {} MB live", it >> 20)).unwrap_or_default();
    eprintln!("memory after {step}: {} MB resident{live}", resident().unwrap_or(0));
}

/// Writes what the daemon holds now that `step` is done to the directory `RA_JIT_MEMORY` names.
pub(crate) fn report(db: &mut RootDatabase, step: &str) -> anyhow::Result<()> {
    checkpoint(step);
    let Some(dir) = report_dir() else { return Ok(()) };
    std::fs::create_dir_all(&dir)?;
    write_heap_profile(&dir.join(format!("{step}.prof")))?;
    std::fs::write(dir.join(format!("{step}.tsv")), ingredients(db))?;
    Ok(())
}

/// [`report`] once the daemon has answered a request, as `request-N`, N counting from 1.
pub(crate) fn served(db: &mut RootDatabase) -> anyhow::Result<()> {
    static SERVED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    if report_dir().is_none() {
        return Ok(());
    }
    let served = SERVED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    report(db, &format!("request-{served}"))
}

/// A line for every struct and query of the database: how many there are and the bytes of
/// their fields, of the database's own records about them and, where the ingredient can
/// tell, of what the fields own on the heap.
fn ingredients(db: &mut RootDatabase) -> String {
    let db: &mut dyn salsa::Database = db;
    let usage = db.memory_usage();
    let structs = usage.structs.iter().map(|it| ("struct", it.debug_name(), it));
    let queries = usage.queries.iter().map(|(&name, it)| ("query", name, it));
    let mut rows: Vec<_> = structs.chain(queries).collect();
    rows.sort_by_key(|(_, _, it)| std::cmp::Reverse(it.size_of_fields() + it.size_of_metadata()));
    let mut table = String::from("kind\tcount\tfields\tmetadata\theap\tname\n");
    for (kind, name, it) in rows {
        let heap = it.heap_size_of_fields().map(|it| it.to_string()).unwrap_or_default();
        _ = writeln!(
            table,
            "{kind}\t{}\t{}\t{}\t{heap}\t{name}",
            it.count(),
            it.size_of_fields(),
            it.size_of_metadata(),
        );
    }
    table
}

#[cfg(feature = "heap-profile")]
fn write_heap_profile(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::ffi::OsStrExt as _;

    // SAFETY: `opt.prof` is a `bool`.
    if !unsafe { tikv_jemalloc_ctl::raw::read::<bool>(b"opt.prof\0") }.unwrap_or(false) {
        return Ok(());
    }
    // A profile has the addresses of the code that allocated, and on macOS nothing of where
    // the program was loaded, which `harness/heapfold.py` needs to name them.
    #[cfg(target_os = "macos")]
    {
        // SAFETY: image 0 is the program itself, which is loaded for as long as it runs.
        let header = unsafe { libc::_dyld_get_image_header(0) } as usize;
        std::fs::write(path.with_extension("load"), format!("{header:#x}\n"))?;
    }
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: `prof.dump` takes a C string, which `path` is for the length of the call.
    unsafe { tikv_jemalloc_ctl::raw::write(b"prof.dump\0", path.as_ptr()) }
        .map_err(|e| anyhow::anyhow!("writing the heap profile: {e}"))
}

#[cfg(not(feature = "heap-profile"))]
fn write_heap_profile(_: &Path) -> anyhow::Result<()> {
    Ok(())
}
