//! PAM-backed login with an admin-group authorization check.

use std::ffi::{CStr, CString};

use anyhow::Context;
use pam_client::conv_mock::Conversation;
use pam_client::{Context as PamContext, ConversationHandler, ErrorCode, Flag};

use super::config::Config;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid username or password")]
    InvalidCredentials,
    #[error("user is not allowed to manage this system")]
    NotAuthorized,
    #[error("password change required before login")]
    PasswordChangeRequired,
    #[error("new password rejected: {0}")]
    PasswordRejected(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

/// Authenticate via PAM and check admin group membership (blocking).
pub async fn login(config: &Config, username: &str, password: &str) -> Result<(), AuthError> {
    let pam_service = config.pam_service.clone();
    let admin_group = config.admin_group.clone();
    let username = username.to_owned();
    let password = password.to_owned();

    tokio::task::spawn_blocking(move || {
        pam_authenticate(&pam_service, &username, &password)?;
        if !is_group_member(&username, &admin_group)? {
            return Err(AuthError::NotAuthorized);
        }
        Ok(())
    })
    .await
    .map_err(|join_error| anyhow::anyhow!(join_error))?
}

fn pam_authenticate(service: &str, username: &str, password: &str) -> Result<(), AuthError> {
    let mut context = PamContext::new(
        service,
        Some(username),
        Conversation::with_credentials(username, password),
    )
    .context("cannot initialize PAM")?;

    // Treat authentication failures uniformly to prevent user enumeration.
    context.authenticate(Flag::NONE).map_err(|error| {
        tracing::info!(username, %error, "PAM rejected login");
        AuthError::InvalidCredentials
    })?;

    // A fresh admin account (see foyer-base-users) has its password aged out
    // so pam_unix forces a change; the credentials themselves were correct,
    // so this must stay distinguishable from InvalidCredentials or the caller
    // has no way to route the user to a change-password flow.
    context.acct_mgmt(Flag::NONE).map_err(|error| {
        if error.code() == ErrorCode::NEW_AUTHTOK_REQD {
            AuthError::PasswordChangeRequired
        } else {
            tracing::info!(username, %error, "PAM rejected login");
            AuthError::InvalidCredentials
        }
    })
}

/// Authenticate with the current password, then use PAM's `chauthtok` to set
/// a new one (blocking). Used to clear a forced/expired password (see
/// foyer-base-users) from an unauthenticated context, since the caller can't
/// have a session yet if their password is expired.
pub async fn change_expired_password(
    config: &Config,
    username: &str,
    current_password: &str,
    new_password: &str,
) -> Result<(), AuthError> {
    let pam_service = config.pam_service.clone();
    let username = username.to_owned();
    let current_password = current_password.to_owned();
    let new_password = new_password.to_owned();

    tokio::task::spawn_blocking(move || {
        pam_change_password(&pam_service, &username, &current_password, &new_password)
    })
    .await
    .map_err(|join_error| anyhow::anyhow!(join_error))?
}

fn pam_change_password(
    service: &str,
    username: &str,
    current_password: &str,
    new_password: &str,
) -> Result<(), AuthError> {
    let mut context = PamContext::new(
        service,
        Some(username),
        ChangePasswordConversation {
            current_password: current_password.to_owned(),
            new_password: new_password.to_owned(),
        },
    )
    .context("cannot initialize PAM")?;

    // Treat authentication failures uniformly to prevent user enumeration.
    context.authenticate(Flag::NONE).map_err(|error| {
        tracing::info!(username, %error, "PAM rejected current password");
        AuthError::InvalidCredentials
    })?;

    // Not every account reaching this endpoint is necessarily expired (e.g. a
    // retry after an earlier failed attempt already cleared it); either way
    // chauthtok below is what actually matters.
    let _ = context.acct_mgmt(Flag::NONE);

    context.chauthtok(Flag::CHANGE_EXPIRED_AUTHTOK).map_err(|error| {
        tracing::info!(username, %error, "PAM rejected new password");
        AuthError::PasswordRejected(error.to_string())
    })
}

/// Answers a PAM conversation spanning both `authenticate()` and
/// `chauthtok()` with two known passwords: prompts mentioning "new" (the
/// "New password"/"Retype new password" prompts pam_unix and pam_cracklib
/// issue) get the new password, everything else — including plain
/// `authenticate()`'s generic "Password:" prompt and chauthtok's
/// "(current) UNIX password:" prompt — gets the current password.
///
/// `conv_mock::Conversation` (used for plain `authenticate()` in `login`
/// above) can't be reused here since it always echoes back a single fixed
/// password, and this handler needs to serve both the current and new
/// password depending on which phase of the conversation is asking.
struct ChangePasswordConversation {
    current_password: String,
    new_password: String,
}

impl ConversationHandler for ChangePasswordConversation {
    fn prompt_echo_on(&mut self, _prompt: &CStr) -> Result<CString, ErrorCode> {
        Err(ErrorCode::CONV_ERR)
    }

    fn prompt_echo_off(&mut self, prompt: &CStr) -> Result<CString, ErrorCode> {
        let prompt = prompt.to_string_lossy().to_lowercase();
        let answer = if prompt.contains("new") {
            &self.new_password
        } else {
            &self.current_password
        };
        CString::new(answer.as_str()).map_err(|_| ErrorCode::CONV_ERR)
    }

    fn text_info(&mut self, msg: &CStr) {
        tracing::debug!(msg = %msg.to_string_lossy(), "PAM info");
    }

    fn error_msg(&mut self, msg: &CStr) {
        tracing::debug!(msg = %msg.to_string_lossy(), "PAM error");
    }
}

fn is_group_member(username: &str, group_name: &str) -> Result<bool, AuthError> {
    let group = nix::unistd::Group::from_name(group_name)
        .context("group lookup failed")?
        .with_context(|| format!("admin group '{group_name}' does not exist"))?;
    let user = nix::unistd::User::from_name(username)
        .context("user lookup failed")?
        .context("user vanished after PAM authentication")?;
    let username_c = CString::new(username).context("invalid username")?;
    let groups =
        nix::unistd::getgrouplist(&username_c, user.gid).context("getgrouplist failed")?;
    Ok(groups.contains(&group.gid))
}
