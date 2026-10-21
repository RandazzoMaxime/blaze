#!/usr/bin/env python3
"""heap.py FOLDED [ROWS]: attributes a heap profile to salsa queries and to ra-jit's own structures.

FOLDED is what `jeprof --collapsed` makes of a profile the daemon wrote (on macOS, what
`heapfold.py` makes of it): build it with `--features heap-profile`, run it with
`RA_JIT_MEMORY=DIR _RJEM_MALLOC_CONF=prof:true`.

Each allocation goes to the innermost salsa ingredient on its stack: a query's `value` (what the
memo keeps, which an `lru` can let go of), its `record` (the dependencies and revisions salsa keeps
of the memo, which stay when the value goes), an `interned` value, or a tracked struct or input.
What no ingredient made goes to ra-jit's own structures by the function that allocated it.
"""
import collections, re, sys

INGREDIENT = re.compile(r"salsa::(function|interned|input|tracked_struct)::IngredientImpl<(.+?)>>::\w+")
EXECUTE = re.compile(r"<(.+?) as salsa::function::Configuration>::execute")
RECORD = ("pop_into_revisions", "OriginAndExtra", "LazyMemoEntries", "insert_memo", "MemoTable",
          "QueryRevisions", "add_read", "report_tracked_read")
OWN = [("symbol table", "SymbolTable"), ("vfs / file loading", "vfs::"), ("vfs / file loading", "load_cargo"),
       ("project model", "project_model"), ("splice", "ra_jit::splice"), ("checkouts", "ra_jit::Checkouts"),
       ("object emission", "cranelift_object"), ("object emission", "object::write"),
       ("codegen driver (outside queries)", "hir_ty_codegen::"), ("ra-jit main (other)", "ra_jit::")]


def short(name):
    name = re.sub(r"::\{closure#\d+\}", "", name)
    name = name.replace("::_::", "::")
    name = re.sub(r"_?Configuration_?$", "", name)
    return re.sub(r"::\w+_shim$", "", name)


def bucket(frames):
    record = False
    for frame in reversed(frames):
        record |= any(it in frame for it in RECORD)
        if m := INGREDIENT.search(frame):
            kind = {"function": "record" if record else "value"}.get(m[1], m[1])
            return kind, short(m[2])
        if m := EXECUTE.search(frame):
            return "record" if record else "value", short(m[1])
    for frame in reversed(frames):
        for label, needle in OWN:
            if needle in frame:
                return "own", label
    return "other", next((f for f in frames[::-1] if "::" in f and "alloc" not in f and "prof_" not in f),
                         frames[-1])[:90]


def main():
    totals = collections.Counter()
    for line in open(sys.argv[1]):
        stack, _, size = line.rstrip().rpartition(" ")
        totals[bucket(stack.split(";"))] += int(size)
    whole = sum(totals.values())
    kinds = collections.Counter()
    for (kind, _), size in totals.items():
        kinds[kind] += size
    print(f"total sampled live: {whole / 2**20:.0f} MB")
    for kind, size in kinds.most_common():
        print(f"{kind}: {size / 2**20:.0f} MB")
    top = int(sys.argv[2]) if len(sys.argv) > 2 else 45
    for (kind, name), size in totals.most_common(top):
        print(f"{size / 2**20:8.0f} MB {100 * size / whole:5.1f}%  {kind:8} {name[:140]}")


if __name__ == "__main__":
    main()
