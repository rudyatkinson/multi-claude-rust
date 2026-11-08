//! Accounts: create, sync, token, status, delete (`src/main/profiles.ts`).
//!
//! The service owns `config.json` (through its [`ConfigStore`], see [`ProfileService::config`])
//! and the tokens of token accounts. What it needs from other services comes in through hooks
//! that the integration sets once at startup:
//!
//! - [`set_cli`](ProfileService::set_cli): `claude auth status` / `claude auth logout` for an
//!   account (with the account's environment). Without it, statuses are not refreshed and logout
//!   says claude was not found.
//! - [`set_close_sessions`](ProfileService::set_close_sessions): closes an account's sessions;
//!   called before a logout and before a delete.
//! - [`set_on_changed`](ProfileService::set_on_changed): called after every change of the
//!   account list or an account, after `profiles:changed` went out. The integration rewrites the
//!   launchers there ([`ProfileService::refresh_launchers`] with the current claude path).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use mc_core::error::{AppError, AppResult};
use mc_core::env::build_profile_env;
use mc_core::folders::{pinned_account, set_folder_pin};
use mc_core::slug::{email_hint, free_slug, is_inside, oauth_token, profile_color, profile_name, slugify};
use mc_core::types::{
    AccountInfo, AppConfig, AuthKind, CreateProfileInput, Profile, ProfilePatch, ProfileView, ShareConfig, SyncResult,
    PROFILE_COLORS,
};
use tauri::AppHandle;

use crate::config_store::{now_ms, ConfigStore};
use crate::events;
use crate::launchers::write_launchers;
use crate::mcp_servers::BoxFuture;
use crate::paths::Paths;
use crate::sharing::{self, MODEL_KEYS, SHARED_DIRS, SHARED_FILES};
use crate::token_vault::{TokenStore, TokenVault, TOKEN_MARKER};

#[cfg(test)]
mod tests;

/// The id of the main account, which the service makes when the config has no default account.
pub const MAIN_ID: &str = "main";

/// `claude auth ...` for an account, as the integration runs it (`src/main/claudeCli.ts`).
pub trait AccountCli: Send + Sync {
    /// Whether there is a claude binary to ask.
    fn available(&self) -> bool;
    /// `claude auth status --json` with the account's environment. Failures come back as a
    /// logged-out status with `error` set, never as an error.
    fn status(&self, profile: Profile) -> BoxFuture<'static, AccountInfo>;
    /// `claude auth logout` with the account's environment. It does not fail.
    fn logout(&self, profile: Profile) -> BoxFuture<'static, ()>;
}

type IdHook = Arc<dyn Fn(&str) + Send + Sync>;
type ChangedHook = Arc<dyn Fn() + Send + Sync>;
type TrashFn = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

#[derive(Default)]
struct Hooks {
    cli: Option<Arc<dyn AccountCli>>,
    close_sessions: Option<IdHook>,
    on_changed: Option<ChangedHook>,
    trash: Option<TrashFn>,
}

fn not_found() -> AppError {
    AppError::refused("Profile not found")
}

fn find_mut<'a>(config: &'a mut AppConfig, id: &str) -> AppResult<&'a mut Profile> {
    config.profiles.iter_mut().find(|p| p.id == id).ok_or_else(not_found)
}

fn main_profile() -> Profile {
    Profile {
        id: MAIN_ID.to_owned(),
        name: "Main".to_owned(),
        slug: "main".to_owned(),
        color: PROFILE_COLORS[0].to_owned(),
        is_default: true,
        config_dir: None,
        share: ShareConfig::default(),
        email_hint: None,
        auth: None,
        token_enc: None,
        last_sync: None,
        account: None,
        share_memory: None,
        extra: Default::default(),
    }
}

/// The accounts.
pub struct ProfileService {
    app: Option<AppHandle>,
    paths: Paths,
    config: Arc<ConfigStore>,
    tokens: Arc<dyn TokenStore>,
    hooks: RwLock<Hooks>,
    /// Makes one account at a time, so two cannot get the same slug.
    creating: Mutex<()>,
}

impl ProfileService {
    /// The service of the app: `config.json` under `root`, tokens in the OS credential store. Adds
    /// the main account when the config has none.
    pub fn new(app: AppHandle, root: PathBuf) -> Self {
        let config = Arc::new(ConfigStore::new(root.clone()));
        Self::with_parts(Some(app), Paths::new(root), config, Arc::new(TokenVault::new()))
    }

    /// A service from its parts. Without an app handle no events go out (tests).
    pub fn with_parts(app: Option<AppHandle>, paths: Paths, config: Arc<ConfigStore>, tokens: Arc<dyn TokenStore>) -> Self {
        let service = ProfileService { app, paths, config, tokens, hooks: RwLock::new(Hooks::default()), creating: Mutex::new(()) };
        if let Err(e) = service.ensure_main() {
            log::error!("profiles: could not save the main account: {e}");
        }
        service
    }

    /// The store of `config.json`. There is one per app: use this one for settings, recent
    /// folders and folder pins too.
    pub fn config(&self) -> &Arc<ConfigStore> {
        &self.config
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    // ---------- hooks ----------

    fn hooks(&self) -> std::sync::RwLockReadGuard<'_, Hooks> {
        self.hooks.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_hook(&self, f: impl FnOnce(&mut Hooks)) {
        f(&mut self.hooks.write().unwrap_or_else(PoisonError::into_inner));
    }

    pub fn set_cli(&self, cli: Arc<dyn AccountCli>) {
        self.set_hook(|h| h.cli = Some(cli));
    }

    /// Closes the sessions of an account; called before its logout and before it is deleted.
    pub fn set_close_sessions(&self, f: impl Fn(&str) + Send + Sync + 'static) {
        self.set_hook(|h| h.close_sessions = Some(Arc::new(f)));
    }

    /// Called after every change, once `profiles:changed` has gone out.
    pub fn set_on_changed(&self, f: impl Fn() + Send + Sync + 'static) {
        self.set_hook(|h| h.on_changed = Some(Arc::new(f)));
    }

    /// Replaces moving a deleted account's folder to the trash (tests).
    pub fn set_trash(&self, f: impl Fn(&Path) -> Result<(), String> + Send + Sync + 'static) {
        self.set_hook(|h| h.trash = Some(Arc::new(f)));
    }

    fn cli(&self) -> Option<Arc<dyn AccountCli>> {
        self.hooks().cli.clone().filter(|c| c.available())
    }

    fn close_sessions(&self, id: &str) {
        let hook = self.hooks().close_sessions.clone();
        if let Some(hook) = hook {
            hook(id);
        }
    }

    fn trash_fn(&self) -> TrashFn {
        self.hooks().trash.clone().unwrap_or_else(|| Arc::new(|dir: &Path| trash::delete(dir).map_err(|e| e.to_string())))
    }

    // ---------- reading ----------

    fn ensure_main(&self) -> AppResult<()> {
        if self.config.with(|c| c.profiles.iter().any(|p| p.is_default)) {
            return Ok(());
        }
        self.config.update(|c| c.profiles.insert(0, main_profile()))
    }

    /// All accounts, Main first unless the file says otherwise.
    pub fn list(&self) -> Vec<Profile> {
        self.config.with(|c| c.profiles.clone())
    }

    pub fn get(&self, id: &str) -> Option<Profile> {
        self.config.with(|c| c.profiles.iter().find(|p| p.id == id).cloned())
    }

    fn require(&self, id: &str) -> AppResult<Profile> {
        self.get(id).ok_or_else(not_found)
    }

    fn view_of(&self, profile: Profile, main_dir: &Path) -> ProfileView {
        let sync = sharing::profile_sync_status(&profile, main_dir);
        let mut view = ProfileView::from(profile);
        view.sync = sync;
        view
    }

    /// The accounts as the window sees them: without tokens, with the sync badge of each.
    pub fn views(&self) -> Vec<ProfileView> {
        let main_dir = self.paths.main_claude_dir();
        self.list().into_iter().map(|p| self.view_of(p, &main_dir)).collect()
    }

    pub fn view(&self, id: &str) -> Option<ProfileView> {
        self.get(id).map(|p| self.view_of(p, &self.paths.main_claude_dir()))
    }

    /// The token of a token account; None for a login account, or when the token is not in the
    /// credential store (an account moved from the Electron version: the user enters it again).
    pub fn token_for(&self, id: &str) -> Option<String> {
        let profile = self.get(id)?;
        if profile.auth != Some(AuthKind::Token) || profile.token_enc.as_deref() != Some(TOKEN_MARKER) {
            return None;
        }
        self.tokens.load(id)
    }

    /// True for a token account whose token is not there to use.
    pub fn token_missing(&self, id: &str) -> bool {
        self.get(id).is_some_and(|p| p.auth == Some(AuthKind::Token)) && self.token_for(id).is_none()
    }

    /// `envFor`: the environment for an account's processes (`buildProfileEnv` over this
    /// process's variables), with the account's token for a token account. `dev` also drops the
    /// variables of the development tooling.
    pub fn env_for_with(&self, profile: &Profile, dev: bool) -> BTreeMap<String, String> {
        let base: BTreeMap<String, String> =
            std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect();
        let tokens = &self.tokens;
        build_profile_env(&base, profile, |enc| (enc == TOKEN_MARKER).then(|| tokens.load(&profile.id)).flatten(), dev)
    }

    /// [`env_for_with`](Self::env_for_with), `dev` for a debug build.
    pub fn env_for(&self, profile: &Profile) -> BTreeMap<String, String> {
        self.env_for_with(profile, cfg!(debug_assertions))
    }

    /// The account a folder is pinned to, if that account still exists.
    pub fn pinned_account(&self, cwd: &str) -> Option<String> {
        self.config.with(|c| pinned_account(&c.folder_accounts, cwd, cfg!(windows), &c.profiles))
    }

    // ---------- events ----------

    /// Sends `profiles:changed` with the views.
    pub fn emit_changed(&self) {
        if let Some(app) = &self.app {
            events::emit(app, events::PROFILES_CHANGED, self.views());
        }
    }

    /// `changed()` of the Electron deps: the event, then the integration's hook.
    fn changed(&self) {
        self.emit_changed();
        let hook = self.hooks().on_changed.clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// `refreshLaunchers`: rewrites the `claude-<slug>` launchers. Nothing without a claude path;
    /// a failure is only logged, as the launchers are a convenience.
    pub fn refresh_launchers(&self, claude_path: Option<&str>) {
        let Some(claude) = claude_path.filter(|p| !p.is_empty()) else {
            return;
        };
        if let Err(e) = write_launchers(&self.paths.bin_dir(), claude, &self.list()) {
            log::debug!("launchers: {e}");
        }
    }

    // ---------- changes ----------

    /// Makes an account: its folder under `profiles/`, the shared folders linked and the shared
    /// files copied before claude ever runs in it, and its token in the credential store.
    /// Returns the account and the warnings of the links and copies.
    pub fn create(&self, input: &CreateProfileInput) -> AppResult<(Profile, Vec<String>)> {
        let name = profile_name(&input.name)?;
        let color = profile_color(&input.color)?.to_owned();
        let token = match input.auth {
            Some(AuthKind::Token) => Some(oauth_token(input.token.as_deref().unwrap_or(""))?),
            _ => None,
        };
        let email = match input.email.as_deref() {
            Some(email) if !email.is_empty() && token.is_none() => Some(email_hint(email)?.to_owned()),
            _ => None,
        };
        let pick = |all: &[&str], wanted: &[String]| -> Vec<String> {
            all.iter().filter(|x| wanted.iter().any(|w| w == *x)).map(|x| (*x).to_owned()).collect()
        };
        let share = ShareConfig {
            dirs: pick(&SHARED_DIRS, &input.share_dirs),
            files: pick(&SHARED_FILES, &input.share_files),
            local_settings_keys: if input.keep_model_separate { MODEL_KEYS.iter().map(|k| (*k).to_owned()).collect() } else { Vec::new() },
        };

        let one_at_a_time = self.creating.lock().unwrap_or_else(PoisonError::into_inner);
        let profiles_dir = self.paths.profiles_dir();
        let taken: Vec<String> = self.config.with(|c| c.profiles.iter().map(|p| p.slug.clone()).collect());
        let slug = free_slug(&slugify(&name), &taken, |s| profiles_dir.join(s).exists());
        let id = uuid::Uuid::new_v4().to_string();
        let config_dir = profiles_dir.join(&slug);

        if let Some(token) = &token {
            self.tokens.save(&id, token)?;
        }
        let undo = |made_dir: bool| {
            if token.is_some() {
                let _ = self.tokens.delete(&id);
            }
            if made_dir {
                let _ = sharing::remove_all_links(&config_dir);
                let _ = fs::remove_dir_all(&config_dir);
            }
        };
        if let Err(e) = fs::create_dir_all(&config_dir) {
            undo(false);
            return Err(AppError::io(format!("Could not create the account folder {}: {e}", config_dir.display())));
        }

        // Shared items go in before claude ever runs in the new dir, so it does not create its own first.
        let main_dir = self.paths.main_claude_dir();
        let links = sharing::apply_dir_links(&main_dir, &config_dir, &share.dirs);
        let copied = match sharing::sync_files(&main_dir, &config_dir, &share, None, true) {
            Ok(copied) => copied,
            Err(e) => {
                undo(true);
                return Err(AppError::io(format!("Could not copy the shared files: {e}")));
            }
        };

        let now = now_ms();
        // a brand-new config dir has no login yet; a token account needs none
        let account = AccountInfo {
            logged_in: token.is_some(),
            email: None,
            org_id: None,
            org_name: None,
            subscription_type: None,
            auth_method: token.is_some().then(|| "oauth_token".to_owned()),
            checked_at: now,
            error: None,
        };
        let profile = Profile {
            id: id.clone(),
            name,
            slug,
            color,
            is_default: false,
            config_dir: Some(config_dir.to_string_lossy().into_owned()),
            share,
            email_hint: email,
            auth: token.as_ref().map(|_| AuthKind::Token),
            token_enc: token.as_ref().map(|_| TOKEN_MARKER.to_owned()),
            last_sync: copied.snapshot,
            account: Some(account),
            share_memory: None,
            extra: Default::default(),
        };
        let saved = self.config.update(|c| c.profiles.push(profile.clone()));
        drop(one_at_a_time);
        self.changed();
        saved?;
        let mut warnings = links.warnings;
        warnings.extend(copied.warnings);
        Ok((profile, warnings))
    }

    /// Changes an account's name, color or memory sharing. Nothing changes when a part is refused.
    pub fn update(&self, id: &str, patch: &ProfilePatch) -> AppResult<()> {
        self.config.try_update(|c| {
            let profile = find_mut(c, id)?;
            if let Some(name) = &patch.name {
                profile.name = profile_name(name)?;
            }
            if let Some(color) = &patch.color {
                profile.color = profile_color(color)?.to_owned();
            }
            if let Some(share) = patch.share_memory {
                if profile.is_default {
                    return Err(AppError::refused("The main profile has the memory the others can share."));
                }
                profile.share_memory = share.then_some(true);
            }
            Ok(())
        })?;
        self.changed();
        Ok(())
    }

    /// Replaces the token of a token account.
    pub fn set_token(&self, id: &str, token: &str) -> AppResult<()> {
        let profile = self.require(id)?;
        if profile.auth != Some(AuthKind::Token) {
            return Err(AppError::refused("This account logs in through the browser"));
        }
        let token = oauth_token(token)?;
        self.tokens.save(id, &token)?;
        self.config.try_update(|c| {
            find_mut(c, id)?.token_enc = Some(TOKEN_MARKER.to_owned());
            Ok(())
        })?;
        self.changed();
        Ok(())
    }

    /// Links and copies what the account shares from Main again. Without `force`, nothing is
    /// written when local edits would be lost (`needsConfirm`).
    pub fn sync_from_main(&self, id: &str, force: bool) -> AppResult<SyncResult> {
        let profile = self.require(id)?;
        let Some(dir) = profile.config_dir.as_deref().map(PathBuf::from) else {
            return Err(AppError::refused("The main profile is not synced"));
        };
        let main_dir = self.paths.main_claude_dir();
        let files = sharing::sync_files(&main_dir, &dir, &profile.share, profile.last_sync.as_ref(), force)
            .map_err(|e| AppError::io(e.to_string()))?;
        if files.needs_confirm {
            return Ok(SyncResult { needs_confirm: true, local_changed: files.local_changed, ..Default::default() });
        }
        let links = sharing::apply_dir_links(&main_dir, &dir, &profile.share.dirs);
        let snapshot = files.snapshot.clone();
        // the account may have been deleted meanwhile; then there is nothing to remember
        let saved = self.config.try_update(|c| {
            find_mut(c, id)?.last_sync = snapshot;
            Ok(())
        });
        if !matches!(saved, Err(AppError::Refused(_))) {
            saved?;
        }
        self.changed();
        let mut backed_up = links.backed_up;
        backed_up.extend(files.backed_up);
        let mut warnings = links.warnings;
        warnings.extend(files.warnings);
        Ok(SyncResult { needs_confirm: false, local_changed: files.local_changed, linked: links.linked, copied: files.copied, backed_up, warnings })
    }

    /// Stores what `claude auth status` said about an account. False when the account is gone.
    pub fn set_account(&self, id: &str, info: AccountInfo) -> AppResult<bool> {
        if self.get(id).is_none() {
            return Ok(false);
        }
        self.config.try_update(|c| {
            find_mut(c, id)?.account = Some(info);
            Ok(())
        })?;
        self.changed();
        Ok(true)
    }

    /// Stores a status and returns the warning when another account is signed in to the same one.
    fn apply_status(&self, id: &str, account: AccountInfo) -> Option<String> {
        match self.set_account(id, account.clone()) {
            Ok(true) => {}
            Ok(false) => return None,
            Err(e) => log::warn!("profiles: could not save the status of {id}: {e}"),
        }
        let email = account.email.as_deref().filter(|_| account.logged_in)?;
        let profiles = self.list();
        let me = profiles.iter().find(|p| p.id == id)?;
        let twin = profiles.iter().find(|p| {
            p.id != id
                && p.account.as_ref().is_some_and(|a| a.logged_in && a.email.as_deref() == Some(email) && a.org_id == account.org_id)
        })?;
        Some(format!("\"{}\" and \"{}\" are signed in to the same account ({email}).", me.name, twin.name))
    }

    /// Reads the account from `claude auth status`. Returns a warning when another account is
    /// signed in to the same one. Nothing happens without a claude binary.
    pub async fn refresh_status(&self, id: &str) -> Option<String> {
        let cli = self.cli()?;
        let profile = self.get(id)?;
        let account = cli.status(profile).await;
        self.apply_status(id, account)
    }

    /// Refreshes every account at once. Returns the warnings.
    pub async fn refresh_all(&self) -> Vec<String> {
        let Some(cli) = self.cli() else {
            return Vec::new();
        };
        let asked: Vec<_> = self.list().into_iter().map(|p| (p.id.clone(), tauri::async_runtime::spawn(cli.status(p)))).collect();
        let mut warnings = Vec::new();
        for (id, answer) in asked {
            match answer.await {
                Ok(account) => warnings.extend(self.apply_status(&id, account)),
                Err(e) => log::warn!("profiles: status of {id}: {e}"),
            }
        }
        warnings
    }

    /// Closes the account's sessions, logs it out and reads its status again.
    pub async fn logout(&self, id: &str) -> AppResult<()> {
        let profile = self.require(id)?;
        let cli = self.cli().ok_or_else(|| AppError::refused("claude was not found"))?;
        self.close_sessions(id);
        cli.logout(profile).await;
        self.refresh_status(id).await;
        Ok(())
    }

    /// Deletes an account: its sessions closed, its login removed, the links in its folder removed
    /// (so nothing of Main can be reached), the folder moved to the trash, its token removed.
    pub async fn delete(&self, id: &str) -> AppResult<()> {
        let profile = self.require(id)?;
        let dir = match (&profile.config_dir, profile.is_default) {
            (Some(dir), false) => PathBuf::from(dir),
            _ => return Err(AppError::refused("The main profile cannot be deleted")),
        };
        if !is_inside(&dir, &self.paths.profiles_dir()) {
            return Err(AppError::refused("The profile folder is not where it is expected; nothing was deleted"));
        }

        self.close_sessions(id);
        // a token account has nothing stored that a logout would remove
        let logged_in = profile.account.as_ref().is_some_and(|a| a.logged_in);
        if let Some(cli) = self.cli().filter(|_| logged_in && profile.auth != Some(AuthKind::Token)) {
            cli.logout(profile.clone()).await;
        }

        if dir.exists() {
            let trash = self.trash_fn();
            tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
                // The links must be gone before the dir is, or the delete could reach Main's files through them.
                let links = sharing::remove_all_links(&dir).map_err(|e| AppError::io(format!("Could not remove the shared links: {e}")))?;
                if !links.remaining.is_empty() {
                    return Err(AppError::io(format!(
                        "Could not remove the shared links ({}); the profile was not deleted",
                        links.remaining.join(", ")
                    )));
                }
                trash(&dir).map_err(|e| AppError::io(format!("Could not move the account folder to the trash: {e}")))
            })
            .await
            .map_err(|e| AppError::io(e.to_string()))??;
        }

        let saved = self.config.update(|c| c.profiles.retain(|p| p.id != id));
        if profile.auth == Some(AuthKind::Token) {
            if let Err(e) = self.tokens.delete(id) {
                log::warn!("profiles: {e}");
            }
        }
        self.changed();
        saved
    }

    /// `setFolderAccount`: pins a folder to an account, or removes its pin with None. Pins of
    /// accounts that were deleted since go at the same time. The caller sends `app:changed`.
    pub fn set_folder_account(&self, cwd: &str, profile_id: Option<&str>) -> AppResult<()> {
        self.config.try_update(|c| {
            let exists = |id: &str| c.profiles.iter().any(|p| p.id == id);
            if profile_id.is_some_and(|id| !exists(id)) {
                return Err(not_found());
            }
            let next = set_folder_pin(&c.folder_accounts, cwd, cfg!(windows), profile_id, exists);
            c.folder_accounts = next;
            Ok(())
        })
    }
}
