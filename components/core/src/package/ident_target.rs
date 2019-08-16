use std::{cmp::{Ord,
                Ordering,
                PartialOrd},
          fmt,
          ops::Deref,
          str::FromStr};

use super::{PackageIdent,
            PackageTarget};
use crate::error::{Error,
                   Result};

// A package ident doesn't uniquely identify an artifact; we need the target as well.
//
#[derive(Debug, Clone, Eq, Hash, PartialEq)] // Copy needs PackageIdent::Copy
pub struct PackageIdentTarget {
    // Ideally these would not be pub, but the accessors seem to hit issues with split borrows
    pub ident:  PackageIdent,
    pub target: PackageTarget,
}

impl PackageIdentTarget {
    pub fn new(ident: PackageIdent, target: PackageTarget) -> Self {
        PackageIdentTarget { ident, target }
    }

    // These would be cool, except that they don't work
    //  println!("Fetching {} for {}", p.ident(), p.target()); hits
    //  E0308 or other borrow related errors, probably because they
    //  are opaque and the checker uses the lifetime of the whole
    //  struct and doesn't treat them as disjoint entities.

    //  https://doc.rust-lang.org/nomicon/borrow-splitting.html
    //
    //    pub fn ident(&self) -> PackageIdent { &self.ident }
    //    pub fn target(&self) -> PackageTarget { &self.target }

    /// Generates the name of the hab package (hart) file.
    pub fn archive_name(&self) -> Result<String> {
        self.ident.archive_name_with_target(self.target)
    }
}

// It would be nice if Ident and Target implemented Ord, PartialOrd and PartialEq
impl Ord for PackageIdentTarget {
    fn cmp(&self, other: &Self) -> Ordering {
        let ord = self.ident.by_parts_cmp(&other.ident);
        match ord {
            Ordering::Equal => self.target.deref().cmp(&other.target.deref()),
            _ => ord,
        }
    }
}

impl PartialOrd for PackageIdentTarget {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

impl fmt::Display for PackageIdentTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.ident, self.target)
    }
}

// PackageIdentTargets are ORIGIN/NAME/VERSION/DATE/TARGET
// This isn't ideal; as we would like to have a partial package ident with a target
impl FromStr for PackageIdentTarget {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let items: Vec<&str> = value.split('/').collect();
        let (ident, target) = match items.len() {
            5 => (
                PackageIdent::new(items[0], items[1], Some(items[2]), Some(items[3])),
                PackageTarget::from_str(items[4])
                    .or_else(|_| Err(Error::InvalidPackageIdentTarget(value.to_string())))?,
            ),
            _ => return Err(Error::InvalidPackageIdentTarget(value.to_string())),
        };
        Ok(PackageIdentTarget::new(ident, target))
    }
}
