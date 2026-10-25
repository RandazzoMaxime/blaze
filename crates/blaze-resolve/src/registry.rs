//! What the resolver asks of the sources: cargo 1.99.0's (5f94df4789) `Registry` trait
//! (`src/workspace/registry.rs`), `QueryKind` (`src/sources/source.rs`) and `IndexSummary`
//! (`src/sources/registry/index/mod.rs`).

use rustc_hash::FxHashMap;
use std::task::Poll;

use anyhow::Result;

use crate::dependency::Dependency;
use crate::interning::InternedString;
use crate::package_id::PackageId;
use crate::source_id::SourceId;
use crate::summary::Summary;

/// How a query matches.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum QueryKind {
    /// Summaries `Dependency::matches`, yanked ones included and marked.
    Exact,
    /// Like `Exact`, with the versions a registry would reject (yanked, unsupported, invalid)
    /// returned for messages.
    RejectedVersions,
    /// Every summary of the name and of names that normalize alike (`-` and `_`), for "did
    /// you mean" messages.
    AlternativeNames,
    /// Matches in every way, with the name normalized as the source defines.
    Normalized,
}

/// One version a source has for a name, and whether it may be chosen.
#[derive(Debug, Clone)]
pub enum IndexSummary {
    Candidate(Summary),
    /// Yanked in its registry: chosen only when a lock file or `[patch]` has it, or
    /// `cargo update --precise` asks for it.
    Yanked(Summary),
    /// Offline, and its `.crate` is not downloaded.
    Offline(Summary),
    /// From an index schema newer than this one reads (the version is the schema's).
    Unsupported(Summary, u32),
    /// A supported schema with an error in the entry.
    Invalid(Summary),
}

impl IndexSummary {
    pub fn summary(&self) -> &Summary {
        match self {
            IndexSummary::Candidate(sum)
            | IndexSummary::Yanked(sum)
            | IndexSummary::Offline(sum)
            | IndexSummary::Unsupported(sum, _)
            | IndexSummary::Invalid(sum) => sum,
        }
    }

    pub fn into_summary(self) -> Summary {
        match self {
            IndexSummary::Candidate(sum)
            | IndexSummary::Yanked(sum)
            | IndexSummary::Offline(sum)
            | IndexSummary::Unsupported(sum, _)
            | IndexSummary::Invalid(sum) => sum,
        }
    }

    pub fn map_summary(self, f: impl Fn(Summary) -> Summary) -> Self {
        match self {
            IndexSummary::Candidate(s) => IndexSummary::Candidate(f(s)),
            IndexSummary::Yanked(s) => IndexSummary::Yanked(f(s)),
            IndexSummary::Offline(s) => IndexSummary::Offline(f(s)),
            IndexSummary::Unsupported(s, v) => IndexSummary::Unsupported(f(s), v),
            IndexSummary::Invalid(s) => IndexSummary::Invalid(f(s)),
        }
    }

    pub fn package_id(&self) -> PackageId {
        self.summary().package_id()
    }

    pub fn is_yanked(&self) -> bool {
        matches!(self, IndexSummary::Yanked(..))
    }

    pub fn is_offline(&self) -> bool {
        matches!(self, IndexSummary::Offline(..))
    }
}

/// The sources as the resolver sees them, with `[patch]`, `[replace]` and source replacement
/// already applied by whoever implements it.
///
/// A query that needs the network may answer `Poll::Pending` and fetch in the background;
/// the resolver then asks everything else it can, calls [`Registry::block_until_ready`] and
/// starts again, so a whole graph's index files are fetched at once.
pub trait Registry {
    /// Calls `f` with each summary of `dep`'s name that matches it as `kind` says.
    fn query(
        &mut self,
        dep: &Dependency,
        kind: QueryKind,
        f: &mut dyn FnMut(IndexSummary),
    ) -> Poll<Result<()>>;

    fn query_vec(&mut self, dep: &Dependency, kind: QueryKind) -> Poll<Result<Vec<IndexSummary>>> {
        let mut ret = Vec::new();
        self.query(dep, kind, &mut |s| ret.push(s)).map_ok(|()| ret)
    }

    /// Waits for every query that answered `Pending`.
    fn block_until_ready(&mut self) -> Result<()>;

    /// How to name the source in messages.
    fn describe_source(&self, source: SourceId) -> String;

    /// Whether `[source]` replaces this source with another.
    fn is_replaced(&self, source: SourceId) -> bool;
}

/// A registry held in memory: the resolver's tests, and sources already read.
#[derive(Default, Debug, Clone)]
pub struct MemRegistry {
    packages: FxHashMap<InternedString, Vec<IndexSummary>>,
}

impl MemRegistry {
    pub fn new() -> MemRegistry {
        MemRegistry::default()
    }

    pub fn add(&mut self, summary: IndexSummary) {
        self.packages.entry(summary.package_id().name()).or_default().push(summary);
    }
}

impl FromIterator<Summary> for MemRegistry {
    fn from_iter<I: IntoIterator<Item = Summary>>(iter: I) -> Self {
        let mut registry = MemRegistry::new();
        for summary in iter {
            registry.add(IndexSummary::Candidate(summary));
        }
        registry
    }
}

impl Registry for MemRegistry {
    fn query(
        &mut self,
        dep: &Dependency,
        kind: QueryKind,
        f: &mut dyn FnMut(IndexSummary),
    ) -> Poll<Result<()>> {
        let normalized = |s: &str| s.replace('-', "_");
        for (name, summaries) in &self.packages {
            let name_matches = match kind {
                QueryKind::AlternativeNames => normalized(name) == normalized(&dep.package_name()),
                _ => *name == dep.package_name(),
            };
            if !name_matches {
                continue;
            }
            for s in summaries {
                let matched = match kind {
                    QueryKind::Exact | QueryKind::RejectedVersions => dep.matches(s.summary()),
                    QueryKind::AlternativeNames | QueryKind::Normalized => true,
                };
                if !matched {
                    continue;
                }
                match s {
                    IndexSummary::Candidate(_) | IndexSummary::Yanked(_) => f(s.clone()),
                    _ if kind == QueryKind::RejectedVersions => f(s.clone()),
                    _ => {}
                }
            }
        }
        Poll::Ready(Ok(()))
    }

    fn block_until_ready(&mut self) -> Result<()> {
        Ok(())
    }

    fn describe_source(&self, source: SourceId) -> String {
        source.to_string()
    }

    fn is_replaced(&self, _source: SourceId) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn summary(name: &str, version: &str) -> Summary {
        let id = PackageId::try_new(name, version, SourceId::crates_io()).unwrap();
        Summary::new(id, vec![], &BTreeMap::new(), None::<&str>, None).unwrap()
    }

    fn versions(r: &mut MemRegistry, dep: &Dependency, kind: QueryKind) -> Vec<String> {
        let Poll::Ready(Ok(found)) = r.query_vec(dep, kind) else { panic!() };
        let mut found: Vec<_> = found
            .iter()
            .map(|s| {
                format!(
                    "{}{}",
                    s.package_id().version(),
                    if s.is_yanked() { " yanked" } else { "" }
                )
            })
            .collect();
        found.sort();
        found
    }

    #[test]
    fn exact_queries_mark_yanked() {
        let mut r = MemRegistry::new();
        r.add(IndexSummary::Candidate(summary("foo", "1.0.0")));
        r.add(IndexSummary::Yanked(summary("foo", "1.1.0")));
        r.add(IndexSummary::Invalid(summary("foo", "1.2.0")));
        r.add(IndexSummary::Candidate(summary("foo", "2.0.0")));
        r.add(IndexSummary::Candidate(summary("foo_bar", "1.0.0")));
        let dep = Dependency::parse("foo", Some("1"), SourceId::crates_io()).unwrap();
        assert_eq!(versions(&mut r, &dep, QueryKind::Exact), ["1.0.0", "1.1.0 yanked"]);
        assert_eq!(
            versions(&mut r, &dep, QueryKind::RejectedVersions),
            ["1.0.0", "1.1.0 yanked", "1.2.0"]
        );

        let dep = Dependency::parse("foo-bar", Some("1"), SourceId::crates_io()).unwrap();
        assert!(versions(&mut r, &dep, QueryKind::Exact).is_empty());
        assert_eq!(versions(&mut r, &dep, QueryKind::AlternativeNames), ["1.0.0"]);
    }
}
