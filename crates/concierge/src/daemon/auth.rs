//! PAM-backed login with an admin-group authorization check.

use std::ffi::CString;

use anyhow::Context;
use pam_client::conv_mock::Conversation;
use pam_client::{Context as PamContext, ErrorCode, Flag};

use super::config::Config;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid username or password")]
    InvalidCredentials,
    #[error("user is not allowed to manage this system")]
    NotAuthorized,
    #[error("password change required before login")]
    PasswordChangeRequired,
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
