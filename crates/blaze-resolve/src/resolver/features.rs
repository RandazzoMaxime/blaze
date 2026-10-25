//! The features asked of a package: from the command line or a dependency. A part of
//! `src/resolver/features.rs` of cargo 1.99.0 (5f94df4789); its feature resolver is not here
//! yet.

use std::collections::BTreeSet;
use std::rc::Rc;

use anyhow::bail;

use super::CargoResult;
use super::types::FeaturesSet;
use crate::summary::FeatureValue;

/// Whether dev-dependencies are wanted.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HasDevUnits {
    Yes,
    No,
}

/// Flag to indicate that target-specific filtering should be disabled.
#[derive(Copy, Clone, PartialEq)]
pub enum ForceAllTargets {
    Yes,
    No,
}

/// Features flags requested for a package.
///
/// This should be cheap and fast to clone, it is used in the resolver for
/// various caches.
///
/// This is split into enum variants because the resolver needs to handle
/// features coming from different places (command-line and dependency
/// declarations), but those different places have different constraints on
/// which syntax is allowed. This helps ensure that every place dealing with
/// features is properly handling those syntax restrictions.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum RequestedFeatures {
    /// Features requested on the command-line with flags.
    CliFeatures(CliFeatures),
    /// Features specified in a dependency declaration.
    DepFeatures {
        /// The `features` dependency field.
        features: FeaturesSet,
        /// The `default-features` dependency field.
        uses_default_features: bool,
    },
}

/// Features specified on the command-line.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct CliFeatures {
    /// Features from the `--features` flag.
    pub features: Rc<BTreeSet<FeatureValue>>,
    /// The `--all-features` flag.
    pub all_features: bool,
    /// Inverse of `--no-default-features` flag.
    pub uses_default_features: bool,
}

impl CliFeatures {
    /// Creates a new `CliFeatures` from the given command-line flags.
    pub fn from_command_line(
        features: &[String],
        all_features: bool,
        uses_default_features: bool,
    ) -> CargoResult<CliFeatures> {
        let features = Rc::new(CliFeatures::split_features(features));
        // Some early validation to ensure correct syntax.
        for feature in features.iter() {
            match feature {
                // Maybe call validate_feature_name here once it is an error?
                FeatureValue::Feature(_) => {}
                FeatureValue::Dep { .. } => {
                    bail!("feature `{}` is not allowed to use explicit `dep:` syntax", feature);
                }
                FeatureValue::DepFeature { dep_feature, .. } => {
                    if dep_feature.contains('/') {
                        bail!("multiple slashes in feature `{}` is not allowed", feature);
                    }
                }
            }
        }
        Ok(CliFeatures { features, all_features, uses_default_features })
    }

    /// Creates a new `CliFeatures` with the given `all_features` setting.
    pub fn new_all(all_features: bool) -> CliFeatures {
        CliFeatures {
            features: Rc::new(BTreeSet::new()),
            all_features,
            uses_default_features: true,
        }
    }

    fn split_features(features: &[String]) -> BTreeSet<FeatureValue> {
        features
            .iter()
            .flat_map(|s| s.split_whitespace())
            .flat_map(|s| s.split(','))
            .filter(|s| !s.is_empty())
            .map(|s| FeatureValue::new(s.into()))
            .collect()
    }
}
