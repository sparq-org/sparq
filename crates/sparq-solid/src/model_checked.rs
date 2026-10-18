//! The model-check token and the one check that makes it.
//!
//! The token's field is private to THIS module, and the only thing in it that builds one is
//! [`crate::PodStore::matching_model`] — so no other module of the crate can mint one, not
//! even the crate root (a private field is visible to the defining module and its
//! descendants, which is why the type cannot live next to the write paths that take it).

use crate::PodStore;

/// Evidence that the model a rebuild is about to install is the model the authorization view
/// in force was built under: [`PodStore::matching_model`] is the only thing that can make
/// one (its field is private to the module that holds that one check, and the type is
/// zero-sized), and every helper that installs a model takes one. So no rebuild path can
/// skip that check, release build or not.
///
/// It is deliberately NOT `Clone` or `Copy`: a token says a check ran, not which model it
/// ran for, so one taken for WAC would otherwise be reusable for an ACP install. Moving it
/// keeps one token to one install, and the helpers that install a view re-run the check
/// anyway — the token is the compile-time half of the guarantee, the re-check the run-time
/// half.
pub(crate) struct ModelChecked(());

impl PodStore {
    /// Refuse a rebuild under the WAC model on a store whose view is ACP, or the other way
    /// round: it would drop the view in force and with it every grant and deny the other
    /// model's rules decide. A store with no view in force (before the first materialization,
    /// or after the view was dropped) is left to the caller, as it is today. An update that
    /// rebuilds no rules is refused just the same: its effects and the facts it stages are
    /// installed over the view in force, which the other model's rules did not build.
    ///
    /// This is the one check, and every entry point that installs a view calls it before it
    /// changes anything: `PodStore::commit_update` (which every `update_as*` passes through,
    /// rebuilding or not), `PodStore::put_acl_inner` (`put_acl`/`put_acl_acp`),
    /// `PodStore::delete_acl_inner`
    /// (`delete_acl`/`delete_acl_acp`) and [`PodStore::rematerialize_after_edit`]. A refusal
    /// therefore leaves the dataset, the stored provenance and the view all as they were.
    ///
    /// It hands back a `ModelChecked`, which every helper that installs a model takes, so a
    /// rebuild cannot reach the view without this check having run — in any build profile.
    pub(crate) fn matching_model(&self, acp: bool) -> Result<ModelChecked, String> {
        match self.installed_acp {
            Some(installed) if installed != acp => Err(format!(
                "this store's authorization view was built under {}: use its {} write entry \
                 point, or materialize the view under {} first; nothing was changed",
                if installed { "ACP" } else { "WAC" },
                if installed { "ACP" } else { "WAC" },
                if acp { "ACP" } else { "WAC" },
            )),
            _ => Ok(ModelChecked(())),
        }
    }
}
