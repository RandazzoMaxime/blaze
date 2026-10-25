//! The `main` of a test executable, as rustc's `--test` writes it: a description of each
//! `#[test]` and `#[bench]` of the crate for libtest, and `test_main_static` over them. ra-jit adds it to
//! the root file of a crate it builds with `cfg(test)`, as a module of its own, and the
//! program starts there instead of in rustc's: a test added, removed or marked otherwise is
//! a build like another.

use std::fmt::Write as _;

use ide_db::RootDatabase;
use syntax::{
    AstNode as _, AstToken as _,
    ast::{self, HasAttrs as _, HasName as _},
};

/// The module's name in the crate's root.
pub(crate) const MODULE: &str = "__ra_jit_test_harness";

/// What the module starts with, after the text of the root file.
const MARK: &str = "\n// ra-jit's test harness, which is not in the file on disk.\n";

/// Where the module, or a proc-macro crate's [`crate::macro_list`] written the same way,
/// starts in `text`.
fn written_at(text: &str) -> Option<usize> {
    [MARK, crate::macro_list::MARK].iter().filter_map(|it| text.find(it)).min()
}

/// The text of the root file without the module.
pub(crate) fn without(text: &str) -> &str {
    written_at(text).map_or(text, |at| &text[..at])
}

/// `text` read anew from a file, with the module `was` had at its end. The module stays in
/// the crate from one revision to the next, where it is written again for the tests that
/// are there now: one that is gone a moment is another module when it is back.
pub(crate) fn kept(was: &str, text: String) -> String {
    match written_at(was) {
        Some(at) => text + &was[at..],
        None => text,
    }
}

/// ra-jit's harness for a crate.
pub(crate) struct Harness {
    /// The module, which starts with [`MARK`].
    pub(crate) text: String,
}

/// One test as libtest describes it.
struct Test {
    /// The function from the crate's root, and the name libtest shows: the same path
    /// without `crate::`.
    path: Vec<String>,
    /// A `#[bench]`, which libtest calls with a `Bencher`.
    bench: bool,
    ignore: bool,
    ignore_message: Option<String>,
    should_panic: ShouldPanic,
    /// Where its name is: the file as rustc is told it, and the line and column of the
    /// start and the end, from 1.
    file: String,
    start: (usize, usize),
    end: (usize, usize),
}

#[derive(Debug)]
enum ShouldPanic {
    No,
    Yes,
    WithMessage(String),
}

/// The module for the tests of `krate`, `None` when a test is not one the module can say as
/// rustc's harness does (its source or its name is not there, or its `should_panic` names no
/// string), which is a bug to fix (no-rustc H, #88).
/// `file_name` is what rustc is told a file is called.
pub(crate) fn module(
    db: &RootDatabase,
    krate: hir::Crate,
    functions: impl IntoIterator<Item = hir::Function>,
    file_name: &dyn Fn(vfs::FileId) -> Option<String>,
) -> Option<Harness> {
    use hir::HasSource as _;
    let edition = krate.edition(db);
    let mut tests = Vec::new();
    for function in functions {
        let bench = function.is_bench(db);
        if !bench && !function.is_test(db) {
            continue;
        }
        let Some(path) = path_of(db, function, edition) else { continue };
        let source = function.source(db)?;
        let mut test = Test {
            path,
            bench,
            ignore: false,
            ignore_message: None,
            should_panic: ShouldPanic::No,
            file: String::new(),
            start: (0, 0),
            end: (0, 0),
        };
        // What `cfg_attr` gives for the crate's cfgs is written as if alone.
        let mut metas: Vec<ast::Meta> = source.value.attrs().filter_map(|it| it.meta()).collect();
        metas.reverse();
        let mut attrs = Vec::new();
        while let Some(meta) = metas.pop() {
            let ast::Meta::CfgAttrMeta(cfg_attr) = meta else {
                attrs.push(meta);
                continue;
            };
            let predicate = cfg::CfgExpr::parse_from_ast(cfg_attr.cfg_predicate()?);
            if krate.cfg(db).check(&predicate) != Some(false) {
                let inner: Vec<ast::Meta> = cfg_attr.metas().collect();
                metas.extend(inner.into_iter().rev());
            }
        }
        for meta in attrs {
            let named = |path: Option<ast::Path>| path.map(|it| it.syntax().text().to_string());
            match meta {
                ast::Meta::PathMeta(meta) => match named(meta.path()).as_deref() {
                    Some("ignore") => test.ignore = true,
                    Some("should_panic") => test.should_panic = ShouldPanic::Yes,
                    _ => {}
                },
                ast::Meta::KeyValueMeta(meta) => {
                    let value = meta.expr().and_then(|it| string_of(&it));
                    match named(meta.path()).as_deref() {
                        Some("ignore") => (test.ignore, test.ignore_message) = (true, value),
                        Some("should_panic") => {
                            test.should_panic = ShouldPanic::WithMessage(value?);
                        }
                        _ => {}
                    }
                }
                // rustc reads no message from `ignore` with a list after it, and ignores the
                // test.
                ast::Meta::TokenTreeMeta(meta) => match named(meta.path()).as_deref() {
                    Some("ignore") => test.ignore = true,
                    Some("should_panic") => {
                        test.should_panic =
                            ShouldPanic::WithMessage(expected(&meta.token_tree()?)?);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        let name = source.value.name()?;
        let range = source.with_value(name.syntax()).original_file_range_rooted(db);
        let file = range.file_id.file_id(db);
        test.file = file_name(file)?;
        let lines = ide_db::line_index(db, file);
        let at = |offset| {
            let it = lines.line_col(offset);
            (it.line as usize + 1, it.col as usize + 1)
        };
        (test.start, test.end) = (at(range.range.start()), at(range.range.end()));
        tests.push(test);
    }
    // rustc lists them by name.
    tests.sort_by(|a, b| a.path.cmp(&b.path));
    let root = krate.root_file(db);
    let test_type = match file_name(root).as_deref().map(std::path::Path::new) {
        Some(it) if it.parent().is_some_and(|it| it.ends_with("src")) => "UnitTest",
        Some(it) if it.parent().is_some_and(|it| it.ends_with("tests")) => "IntegrationTest",
        _ => "Unknown",
    };
    let tested = krate.cfg(db).check_atom(&cfg::CfgAtom::Flag(intern::sym::test));
    Some(Harness { text: text(&tests, test_type, tested) })
}

/// The path of a test from its crate's root, which joined by `::` is the name libtest shows.
/// `None` for a test inside a function, which rustc's harness leaves out too: it can't be
/// named.
fn path_of(
    db: &RootDatabase,
    function: hir::Function,
    edition: syntax::Edition,
) -> Option<Vec<String>> {
    let modules = function.module(db).path_to_root(db);
    let mut path = Vec::new();
    for module in modules.iter().rev().skip(1) {
        path.push(module.name(db)?.display(db, edition).to_string());
    }
    path.push(function.name(db).display(db, edition).to_string());
    Some(path)
}

/// The name libtest shows of a `#[test]` of `krate`, the one `--exact` picks it by.
pub(crate) fn name(
    db: &RootDatabase,
    krate: hir::Crate,
    function: hir::Function,
) -> Option<String> {
    Some(path_of(db, function, krate.edition(db))?.join("::"))
}

/// `tested` is whether the crate has `cfg(test)`, which the module then asks for: a library
/// built with it for its unit tests has its root file in common with the library others link.
fn text(tests: &[Test], test_type: &str, tested: bool) -> String {
    let mut text = String::from(MARK);
    if tested {
        text.push_str("#[cfg(test)]\n");
    }
    text.push_str("#[doc(hidden)]\n#[allow(warnings, clippy::all)]\npub mod ");
    text.push_str(MODULE);
    text.push_str(
        " {\n    extern crate test;\n\n    pub fn main() {\n        \
         const TESTS: &[&test::TestDescAndFn] = &[",
    );
    for at in 0..tests.len() {
        _ = write!(text, "&T{at}, ");
    }
    text.push_str(
        "];\n        if !sandbox::ran(TESTS) {\n            test::test_main_static(TESTS)\n        \
         }\n    }\n",
    );
    text.push_str(SANDBOX);
    for (at, test) in tests.iter().enumerate() {
        let name = test.path.join("::");
        let ignore_message = match &test.ignore_message {
            Some(it) => format!("::core::option::Option::Some({it:?})"),
            None => "::core::option::Option::None".to_owned(),
        };
        let should_panic = match &test.should_panic {
            ShouldPanic::No => "No".to_owned(),
            ShouldPanic::Yes => "Yes".to_owned(),
            ShouldPanic::WithMessage(it) => format!("YesWithMessage({it:?})"),
        };
        let ((start_line, start_col), (end_line, end_col)) = (test.start, test.end);
        _ = write!(
            text,
            "\n    const T{at}: test::TestDescAndFn = test::TestDescAndFn {{\n        \
             desc: test::TestDesc {{\n            \
             name: test::StaticTestName({name:?}),\n            \
             ignore: {ignore},\n            \
             ignore_message: {ignore_message},\n            \
             source_file: {file:?},\n            \
             start_line: {start_line}usize,\n            \
             start_col: {start_col}usize,\n            \
             end_line: {end_line}usize,\n            \
             end_col: {end_col}usize,\n            \
             should_panic: test::ShouldPanic::{should_panic},\n            \
             compile_fail: false,\n            \
             no_run: false,\n            \
             test_type: test::TestType::{test_type},\n        \
             }},\n        \
             testfn: {testfn},\n    \
             }};\n",
            ignore = test.ignore,
            file = test.file,
            testfn = match test.bench {
                true =>
                    format!("test::StaticBenchFn(|b| test::assert_test_result(crate::{name}(b)))"),
                false =>
                    format!("test::StaticTestFn(|| test::assert_test_result(crate::{name}()))"),
            },
        );
    }
    text.push_str("}\n");
    text
}

/// The variable `cargo ra-jit test` names a file in for the harness to run tests each in a
/// sandbox of its own. Its first line is the command that runs a program in one, by its
/// arguments joined by tabs, which the harness gives its own executable after; each line after
/// it is the name of a test to run there ([`crate::cargo::sandboxed`]).
pub(crate) const SANDBOXED: &str = "RA_JIT_TEST_SANDBOX";

/// The part of the harness that runs a test in a process of its own, in a sandbox. libtest
/// runs in the program `cargo ra-jit test` started, as it does without one: it picks the
/// tests, runs them on its threads, judges `#[should_panic]`, keeps their output and prints
/// it. A test the file of [`SANDBOXED`] names is a function there that runs the program
/// again through the sandbox's command, told the test by [`crate::sandbox::RUN`]; that
/// process runs the test alone as libtest would, on a thread with its name and its output
/// kept, and writes on descriptor 3 how it ended and what it printed, which the function
/// gives libtest as its own. A test that ends its process, or crashes it, ends the program
/// the same way, as it would have without the sandbox.
const SANDBOX: &str = r#"
    #[cfg(not(unix))]
    mod sandbox {
        pub fn ran(_: &[&super::test::TestDescAndFn]) -> bool {
            false
        }
    }

    #[cfg(unix)]
    mod sandbox {
        use super::test;
        use ::std::io::{Read as _, Write as _};
        use ::std::os::unix::io::{AsRawFd as _, FromRawFd as _};
        use ::std::os::unix::process::{CommandExt as _, ExitStatusExt as _};

        unsafe extern "C" {
            fn dup(fd: i32) -> i32;
            fn dup2(fd: i32, to: i32) -> i32;
            fn close(fd: i32) -> i32;
        }

        const SPEC: &str = "RA_JIT_TEST_SANDBOX";
        const ONE: &str = "__RA_JIT_TEST";
        const CAPTURE: &str = "__RA_JIT_TEST_CAPTURE";

        pub fn ran(tests: &[&test::TestDescAndFn]) -> bool {
            if let Some(name) = ::std::env::var_os(ONE) {
                one(tests, &name.to_string_lossy());
            }
            let Some(spec) = ::std::env::var_os(SPEC) else { return false };
            unsafe { ::std::env::remove_var(SPEC) };
            let text = ::std::fs::read_to_string(&spec).unwrap_or_default();
            let mut lines = text.lines();
            let command: ::std::vec::Vec<::std::string::String> = lines
                .next()
                .unwrap_or_default()
                .split('\t')
                .filter(|it| !it.is_empty())
                .map(::std::string::String::from)
                .collect();
            let apart: ::std::collections::HashSet<&str> = lines.collect();
            let exe = ::std::env::current_exe().ok().filter(|_| !command.is_empty());
            let owned = tests
                .iter()
                .map(|it| {
                    let desc = it.desc.clone();
                    let run = match it.testfn {
                        test::StaticTestFn(run) => run,
                        _ => unreachable!(),
                    };
                    let name = desc.name.as_slice().to_owned();
                    match &exe {
                        Some(exe) if apart.contains(name.as_str()) => {
                            let (command, exe) = (command.clone(), exe.clone());
                            let testfn = test::DynTestFn(::std::boxed::Box::new(move || {
                                apart_run(&command, &exe, &name)
                            }));
                            test::TestDescAndFn { desc, testfn }
                        }
                        _ => test::TestDescAndFn { desc, testfn: test::StaticTestFn(run) },
                    }
                })
                .collect();
            let args: ::std::vec::Vec<::std::string::String> = ::std::env::args().collect();
            test::test_main(&args, owned, None);
            true
        }

        fn apart_run(
            command: &[::std::string::String],
            exe: &::std::path::Path,
            name: &str,
        ) -> ::std::result::Result<(), ::std::string::String> {
            let (mut said, to) = ::std::io::pipe().map_err(|it| it.to_string())?;
            let mut run = ::std::process::Command::new(&command[0]);
            run.args(&command[1..]).arg(exe).env(ONE, name);
            // libtest keeps what a test prints on its thread unless told not to.
            let capture = ::std::io::set_output_capture(None);
            ::std::io::set_output_capture(capture.clone());
            if capture.is_some() {
                run.env(CAPTURE, "1");
            }
            let fd = to.as_raw_fd();
            unsafe {
                run.pre_exec(move || {
                    let done = match fd {
                        3 => {
                            let copy = dup(3);
                            let done = dup2(copy, 3);
                            close(copy);
                            done
                        }
                        _ => dup2(fd, 3),
                    };
                    match done < 0 {
                        true => Err(::std::io::Error::last_os_error()),
                        false => Ok(()),
                    }
                });
            }
            let mut child = run.spawn().map_err(|it| format!("the test's sandbox did not start: {it}"))?;
            drop(run);
            drop(to);
            let mut bytes = ::std::vec::Vec::new();
            _ = said.read_to_end(&mut bytes);
            let status = child.wait().map_err(|it| it.to_string())?;
            let mut rest = &bytes[..];
            let ended = take_byte(&mut rest);
            let (message, string) = match ended {
                Some(b'e') => (take(&mut rest), true),
                Some(b'p') => {
                    let string = take_byte(&mut rest) == Some(1);
                    (take(&mut rest), string)
                }
                _ => (None, false),
            };
            if let Some(sink) = &capture {
                sink.lock().unwrap_or_else(|it| it.into_inner()).extend_from_slice(rest);
            }
            let message = message.map(|it| ::std::string::String::from_utf8_lossy(&it).into_owned());
            match (ended, message) {
                (Some(b'o'), _) => Ok(()),
                (Some(b'e'), Some(message)) => Err(message),
                (Some(b'p'), Some(message)) if string => ::std::panic::resume_unwind(::std::boxed::Box::new(message)),
                (Some(b'p'), _) => ::std::panic::resume_unwind(::std::boxed::Box::new(())),
                _ => {
                    let code = status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
                    ::std::process::exit(code)
                }
            }
        }

        fn take_byte(from: &mut &[u8]) -> ::std::option::Option<u8> {
            let (&first, rest) = from.split_first()?;
            *from = rest;
            Some(first)
        }

        fn take(from: &mut &[u8]) -> ::std::option::Option<::std::vec::Vec<u8>> {
            let at = from.get(..4)?;
            let len = u32::from_le_bytes([at[0], at[1], at[2], at[3]]) as usize;
            let bytes = from.get(4..4 + len)?.to_vec();
            *from = &from[4 + len..];
            Some(bytes)
        }

        fn put(to: &mut ::std::vec::Vec<u8>, bytes: &[u8]) {
            to.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            to.extend_from_slice(bytes);
        }

        fn one(tests: &[&test::TestDescAndFn], name: &str) -> ! {
            let capture = ::std::env::var_os(CAPTURE).is_some();
            unsafe {
                ::std::env::remove_var(ONE);
                ::std::env::remove_var(CAPTURE);
            }
            // A copy that what the test starts does not get: descriptor 3 is closed.
            let three = unsafe { ::std::fs::File::from_raw_fd(3) };
            let to = three.try_clone();
            drop(three);
            let Ok(mut to) = to else { ::std::process::exit(101) };
            let found = tests.iter().find(|it| it.desc.name.as_slice() == name);
            let Some(run) = found.and_then(|it| match it.testfn {
                test::StaticTestFn(run) => Some(run),
                _ => None,
            }) else {
                eprintln!("ra-jit: no test is named {name:?}");
                ::std::process::exit(101)
            };
            let sink = ::std::sync::Arc::new(::std::sync::Mutex::new(::std::vec::Vec::new()));
            let kept = sink.clone();
            let ended = ::std::thread::Builder::new()
                .name(name.to_owned())
                .spawn(move || {
                    if capture {
                        ::std::io::set_output_capture(Some(kept));
                    }
                    __rust_begin_short_backtrace(run)
                })
                .map(|it| it.join());
            let mut said = ::std::vec::Vec::new();
            match ended {
                Ok(Ok(Ok(()))) => said.push(b'o'),
                Ok(Ok(Err(message))) => {
                    said.push(b'e');
                    put(&mut said, message.as_bytes());
                }
                Ok(Err(payload)) => {
                    said.push(b'p');
                    let text = payload
                        .downcast_ref::<::std::string::String>()
                        .cloned()
                        .or_else(|| payload.downcast_ref::<&str>().map(|it| it.to_string()));
                    said.push(text.is_some() as u8);
                    put(&mut said, text.unwrap_or_default().as_bytes());
                }
                Err(error) => {
                    said.push(b'e');
                    put(&mut said, error.to_string().as_bytes());
                }
            }
            said.extend_from_slice(&sink.lock().unwrap_or_else(|it| it.into_inner()));
            _ = to.write_all(&said);
            ::std::process::exit(0)
        }

        // The frame libtest's is, below which a short backtrace leaves frames out.
        #[inline(never)]
        fn __rust_begin_short_backtrace<T, F: FnOnce() -> T>(f: F) -> T {
            let result = f();
            ::std::hint::black_box(result)
        }
    }
"#;

/// The value of a string literal.
fn string_of(expr: &ast::Expr) -> Option<String> {
    let ast::Expr::Literal(literal) = expr else { return None };
    let ast::LiteralKind::String(string) = literal.kind() else { return None };
    Some(string.value().ok()?.into_owned())
}

/// The message of `should_panic(expected = "...")`. `None` for any other argument: rustc
/// takes no other.
fn expected(tree: &ast::TokenTree) -> Option<String> {
    let tokens: Vec<_> = tree
        .syntax()
        .children_with_tokens()
        .filter_map(|it| it.into_token())
        .filter(|it| !it.kind().is_trivia())
        .collect();
    match tokens.as_slice() {
        [open, name, eq, value, close]
            if open.text() == "("
                && name.text() == "expected"
                && eq.text() == "="
                && close.text() == ")" =>
        {
            Some(ast::String::cast(value.clone())?.value().ok()?.into_owned())
        }
        _ => None,
    }
}
