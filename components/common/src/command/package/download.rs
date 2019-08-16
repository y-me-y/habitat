//! Installs a Habitat package from a [depot](../depot).
//!
//! # Examples
//!
//! ```bash
//! $ hab pkg install core/redis
//! ```
//!
//! Will install `core/redis` package from a custom depot:
//!
//! ```bash
//! $ hab pkg install core/redis/3.0.1 redis -u http://depot.co:9633
//! ```
//!
//! This would install the `3.0.1` version of redis.
//!
//! # Internals
//!
//! * Download the artifact
//! * Verify it is un-altered
//! * Unpack it

use std::{collections::HashSet,
          path::{Path,
                 PathBuf},
          time::Duration};

use crate::{api_client::{self,
                         BoxedClient,
                         Client,
                         Error::APIError},
            hcore::{self,
                    crypto::{artifact,
                             keys::parse_name_with_rev,
                             SigKeyPair},
                    fs::{cache_artifact_path,
                         cache_key_path},
                    package::{Identifiable,
                              PackageArchive,
                              PackageIdent,
                              PackageIdentTarget,
                              PackageTarget},
                    ChannelIdent}};

use reqwest::StatusCode;
use retry::{delay,
            retry};

use crate::{error::{Error,
                    Result},
            ui::{Status,
                 UIWriter}};

pub const RETRIES: usize = 5;
pub const RETRY_WAIT: Duration = Duration::from_millis(3000);

#[derive(Debug, Eq, PartialEq)]
pub enum InstallMode {
    Online,
    Offline,
}

impl Default for InstallMode {
    fn default() -> Self { InstallMode::Online }
}

/// Represents a fully-qualified Package Identifier, meaning that the normally optional version and
/// release package coordinates are guaranteed to be set. This fully-qualified-ness is checked on
/// construction and as the underlying representation is immutable, this state does not change.

/// Download a Habitat package.
///
/// If an `PackageIdentTarget` is given, we retrieve the package
/// from the specified Builder `url`. Providing a fully-qualified
/// identifer will result in that exact package being installed
/// (regardless of `channel`). Providing a partially-qualified
/// identifier will result in the installation of latest appropriate
/// release from the given `channel`.
///
/// Any dependencies of will be retrieved from Builder (if they're not
/// already cached locally).
///
/// At the end of this function, the specified package and all its
/// dependencies will be downloaded on the system.

/// Note: it's worth investigating whether
/// LocalPackageUsage makes sense here
/// Also, in the future we may want to accept an alternate builder to 'filter' what we pull down by
/// That would greatly optimize the 'sync' to on prem builder case, as we could point to that
/// and only fetch what we don't already have.
#[allow(clippy::too_many_arguments)]
pub fn start<U>(ui: &mut U,
                url: &str,
                channel: &ChannelIdent,
                product: &str,
                version: &str,
                idents: Vec<PackageIdent>,
                target: PackageTarget,
                fs_root_path: &Path,
                token: Option<&str>)
                -> Result<()>
    where U: UIWriter
{
    // TODO (CM): rename fs::cache_key_path so the naming is
    // consistent and flows better.
    let key_cache_path = &cache_key_path(Some(fs_root_path));
    debug!("install key_cache_path: {}", key_cache_path.display());

    let artifact_cache_path = &cache_artifact_path(Some(fs_root_path));
    debug!("install artifact_cache_path: {}",
           artifact_cache_path.display());

    let api_client = Client::new(url, product, version, Some(fs_root_path))?;
    let task = DownloadTask { idents,
                              target,
                              api_client,
                              token,
                              channel,
                              artifact_cache_path,
                              key_cache_path };

    let expanded_idents: HashSet<PackageIdentTarget> = task.expand_sources(ui).unwrap();

    debug!("Expanded package count: {}", expanded_idents.len());

    // This could quite reasonably be done in parallel; and probably needs retries
    for package in expanded_idents.iter() {
        task.get_cached_archive(ui, &package)?;
    }

    Ok(())
}

struct DownloadTask<'a> {
    idents: Vec<PackageIdent>,
    target: PackageTarget,
    api_client: BoxedClient,
    token: Option<&'a str>,
    channel: &'a ChannelIdent,
    /// The path to the local artifact cache (e.g., /hab/cache/artifacts)
    artifact_cache_path: &'a Path,
    key_cache_path: &'a Path,
}

impl<'a> DownloadTask<'a> {
    // For each source, use depot to e
    fn expand_sources<T>(&self, ui: &mut T) -> Result<HashSet<PackageIdentTarget>>
        where T: UIWriter
    {
        ui.begin(format!("Preparing to download necessary packages for {} idents",
                         self.idents.len()))?;
        ui.begin(format!("Storing in cache at {:?} ", self.artifact_cache_path))?;
        ui.status(Status::Using, format!("token {:?}", self.token))?;

        let mut expanded_artifacts = HashSet::<PackageIdentTarget>::new();

        for ident in &self.idents {
            match self.determine_latest_from_ident(ui, &ident) {
                Err(_) => {
                    // Probably should be a little more granular with errors; retry 500's etc.
                    ui.status(Status::Missing,
                              format!("{} not found for {} architecture", ident, self.target))?;
                }
                Ok(artifact) => {
                    ui.status(Status::Using,
                              format!("{} as lastest matching {}", artifact, ident))?;
                    self.expand_fully_qualified_dep(ui, &artifact, &mut expanded_artifacts)?;
                }
            }
        }

        ui.status(Status::Found,
                  format!("{} package artifacts, including transitive deps",
                          expanded_artifacts.len()))?;

        Ok(expanded_artifacts)
    }

    // This function and it's sibling in install.rs deserve to be refactored to eke out commonality.
    fn determine_latest_from_ident<T>(&self,
                                      ui: &mut T,
                                      ident: &PackageIdent)
                                      -> Result<PackageIdentTarget>
        where T: UIWriter
    {
        let possible_package = PackageIdentTarget { ident:  ident.clone(),
                                                    target: self.target, };

        if ident.fully_qualified() {
            // If we have a fully qualified package identifier, then our work is done--there can
            // only be *one* package that satisfies a fully qualified identifier.
            Ok(possible_package)
        } else {
            // Unlike in the install command, we always hit the online
            // depot; our purpose is to sync with latest, and falling
            // back to a local package would defeat that. Find the
            // latest package in the proper channel from Builder API,
            ui.status(Status::Determining,
                      format!("latest version of {} in the '{}' channel",
                              &ident, self.channel))?;
            match self.fetch_latest_pkg_ident_in_channel_for(&possible_package, self.channel) {
                Ok(latest_artifact) => Ok(latest_artifact),
                Err(Error::APIClient(APIError(StatusCode::NOT_FOUND, _))) => {
                    self.recommend_channels(ui, &possible_package)?;
                    Err(Error::PackageNotFound("".to_string()))
                }
                Err(e) => {
                    debug!("error fetching ident: {:?}", e);
                    Err(e)
                }
            }
        }
    }

    //
    fn expand_fully_qualified_dep<T>(&self,
                                     ui: &mut T,
                                     package: &PackageIdentTarget,
                                     expanded_idents: &mut HashSet<PackageIdentTarget>)
                                     -> Result<()>
        where T: UIWriter
    {
        // If we've already put this package in the expanded idents
        // (because it is a transitive dep of something we've already
        // looked at), we can stop, our job is done
        if expanded_idents.contains(&package) {
            return Ok(());
        }

        // We fetch the whole artifact; we *could* do this in two passes where we fetch the TDEPS
        // via the API, and go back and download that expanded list as a separate phase.
        // But if we also use local packages as a source of truth for TDEPS doing it this way
        // simplifies the logic, and saves a API call

        // The flip side of this reuse is that we may well want to do API calls and downloads in
        // parallel, and the more complex structure here makes it harder. For parallelism,
        // it would be easier to do it in phases; phase 1: resolve all idents to fully qualified
        // ones, phase 2: expand all tdeps, phase 3: download everything. Each phase could
        // be done pretty straightforwardly as a parallel fork-join operation.

        let mut archive: PackageArchive = self.get_cached_archive(ui, &package)?;

        expanded_idents.insert(package.clone()); // need to figure out how to indicate package and expanded_idents have same lifetime

        // At this point we have the package locally, and can extract the TDEPS
        let dependencies = archive.tdeps()?;
        let dep_count = dependencies.len();
        let expanded_idents_length_before = expanded_idents.len();

        for dependency in dependencies.iter() {
            let dep_package = PackageIdentTarget { ident: dependency.clone(), /* Can I remove
                                                                               * this? */
                                                   target: package.target, };
            expanded_idents.insert(dep_package);
        }

        debug!("Found {} TDEPS, net added {} deps",
               dep_count,
               expanded_idents.len() - expanded_idents_length_before);

        Ok(())
    }

    // This function and it's sibling get_cached_artifact in
    // install.rs deserve to be refactored to eke out commonality.
    /// This ensures the identified package is in the local cache,
    /// verifies it, and returns a handle to the package's metadata.
    fn get_cached_archive<T>(&self,
                             ui: &mut T,
                             package: &PackageIdentTarget)
                             -> Result<PackageArchive>
        where T: UIWriter
    {
        let fetch_artifact = || self.fetch_artifact(ui, package);
        if self.is_artifact_cached(package) {
            debug!("Found {} in artifact cache, skipping remote download",
                   package.ident);
        } else if let Err(err) = retry(delay::Fixed::from(RETRY_WAIT).take(RETRIES), fetch_artifact)
        {
            return Err(Error::DownloadFailed(format!("We tried {} times but \
                                                      could not download {}. \
                                                      Last error was: {}",
                                                     RETRIES, package, err)));
        }

        // At this point the artifact is in the cache...
        let mut artifact = PackageArchive::new(self.cached_artifact_path(package));
        ui.status(Status::Verifying, artifact.ident()?)?;
        self.verify_artifact(ui, package, &mut artifact)?;
        Ok(artifact)
    }

    // This function and it's sibling in install.rs deserve to be refactored to eke out commonality.
    /// Retrieve the identified package from the depot, ensuring that
    /// the artifact is cached locally.
    fn fetch_artifact<T>(&self, ui: &mut T, package: &PackageIdentTarget) -> Result<()>
        where T: UIWriter
    {
        ui.status(Status::Downloading, package)?;
        match self.api_client
                  .fetch_package((&package.ident, package.target),
                                 self.token,
                                 self.artifact_cache_path,
                                 ui.progress())
        {
            Ok(_) => Ok(()),
            Err(api_client::Error::APIError(StatusCode::NOT_IMPLEMENTED, _)) => {
                println!("Host platform or architecture not supported by the targeted depot; \
                          skipping.");
                Ok(())
            }
            Err(e) => Err(Error::from(e)),
        }
    }

    fn fetch_origin_key<T>(&self,
                           ui: &mut T,
                           name_with_rev: &str,
                           token: Option<&str>)
                           -> Result<()>
        where T: UIWriter
    {
        ui.status(Status::Downloading,
                  format!("{} public origin key", &name_with_rev))?;
        let (name, rev) = parse_name_with_rev(&name_with_rev)?;
        self.api_client
            .fetch_origin_key(&name, &rev, token, self.key_cache_path, ui.progress())?;
        ui.status(Status::Cached,
                  format!("{} public origin key", &name_with_rev))?;
        Ok(())
    }

    fn verify_artifact<T>(&self,
                          ui: &mut T,
                          package: &PackageIdentTarget,
                          artifact: &mut PackageArchive)
                          -> Result<()>
        where T: UIWriter
    {
        let artifact_ident = artifact.ident()?;
        if package.ident.as_ref() != &artifact_ident {
            return Err(Error::ArtifactIdentMismatch((artifact.file_name(),
                                                     artifact_ident.to_string(),
                                                     package.to_string())));
        }

        let artifact_target = artifact.target()?;
        if package.target != artifact_target {
            return Err(Error::HabitatCore(hcore::Error::WrongActivePackageTarget(
                package.target,
                artifact_target,
            )));
        }

        let nwr = artifact::artifact_signer(&artifact.path)?;
        if SigKeyPair::get_public_key_path(&nwr, self.key_cache_path).is_err() {
            self.fetch_origin_key(ui, &nwr, self.token)?;
        }

        artifact.verify(&self.key_cache_path)?;
        debug!("Verified {} signed by {}", package, &nwr);
        Ok(())
    }

    // This function and it's sibling in install.rs deserve to be refactored to eke out commonality.
    fn is_artifact_cached(&self, package: &PackageIdentTarget) -> bool {
        self.cached_artifact_path(package).is_file()
    }

    // This function and it's sibling in install.rs deserve to be refactored to eke out commonality.
    /// Returns the path to the location this package would exist at in
    /// the local package cache. It does not mean that the package is
    /// actually *in* the package cache, though.
    fn cached_artifact_path(&self, package: &PackageIdentTarget) -> PathBuf {
        self.artifact_cache_path
            .join(package.archive_name().unwrap())
    }

    // This function and it's sibling in install.rs deserve to be refactored to eke out commonality.
    fn fetch_latest_pkg_ident_in_channel_for(&self,
                                             artifact: &PackageIdentTarget,
                                             channel: &ChannelIdent)
                                             -> Result<PackageIdentTarget> {
        let origin_package =
            self.api_client
                .show_package((&artifact.ident, artifact.target), channel, self.token)?;
        Ok(PackageIdentTarget { ident:  origin_package,
                                target: self.target, })
    }

    // TODO fn: I'm skeptical as to whether we want these warnings all the time. Perhaps it's
    // better to warn that nothing is found and redirect a user to run another standalone
    // `hab pkg ...` subcommand to get more information.
    fn recommend_channels<T>(&self, ui: &mut T, package: &PackageIdentTarget) -> Result<()>
        where T: UIWriter
    {
        if let Ok(recommendations) = self.get_channel_recommendations(&package) {
            if !recommendations.is_empty() {
                ui.warn(format!("No releases of {} exist in the '{}' channel",
                                &package, self.channel))?;
                ui.warn("The following releases were found:")?;
                for r in recommendations {
                    ui.warn(format!("  {} in the '{}' channel", r.1, r.0))?;
                }
            }
        }
        Ok(())
    }

    /// Get a list of suggested package identifiers from all
    /// channels. This is used to generate actionable user feedback
    /// when the desired package was not found in the specified
    /// channel.
    fn get_channel_recommendations(&self,
                                   package: &PackageIdentTarget)
                                   -> Result<Vec<(String, String)>> {
        let mut res = Vec::new();

        let channels = match self.api_client.list_channels(package.ident.origin(), false) {
            Ok(channels) => channels,
            Err(e) => {
                debug!("Failed to get channel list: {:?}", e);
                return Err(Error::PackageNotFound("".to_string()));
            }
        };

        for channel in channels.into_iter().map(ChannelIdent::from) {
            if let Ok(pkg) = self.fetch_latest_pkg_ident_in_channel_for(package, &channel) {
                res.push((channel.to_string(), format!("{}", pkg)));
            }
        }

        Ok(res)
    }
}
