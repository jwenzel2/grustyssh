use std::sync::Arc;
use russh::client;
use russh::{ChannelMsg, Disconnect};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::app::{SshCommand, SshEvent};
use crate::error::AppError;
use crate::models::connection::{AuthMethod, ConnectionProfile};
use crate::ssh::algorithms::preferred_algorithms;
use crate::ssh::handler::ClientHandler;
use crate::ssh::tunnel;
use crate::storage::paths;

/// Spawn an SSH session task.  Returns the command sender for controlling the session.
pub fn spawn_session(
    profile: ConnectionProfile,
    password: Option<Zeroizing<String>>,
    key_passphrase: Option<Zeroizing<String>>,
    event_tx: async_channel::Sender<SshEvent>,
) -> async_channel::Sender<SshCommand> {
    let (cmd_tx, cmd_rx) = async_channel::bounded::<SshCommand>(64);

    let rt = crate::runtime();
    rt.spawn(async move {
        if let Err(e) = run_session(profile, password, key_passphrase, event_tx.clone(), cmd_rx).await {
            let _ = event_tx.send(SshEvent::Error(e.to_string())).await;
            let _ = event_tx
                .send(SshEvent::Disconnected(Some(e.to_string())))
                .await;
        }
    });

    cmd_tx
}

async fn run_session(
    profile: ConnectionProfile,
    password: Option<Zeroizing<String>>,
    key_passphrase: Option<Zeroizing<String>>,
    event_tx: async_channel::Sender<SshEvent>,
    cmd_rx: async_channel::Receiver<SshCommand>,
) -> Result<(), AppError> {
    let config = Arc::new(client::Config {
        preferred: preferred_algorithms(),
        ..Default::default()
    });

    let handler = ClientHandler::new(event_tx.clone());
    let _host_key_accepted = handler.host_key_accepted.clone();
    let _host_key_notify = handler.host_key_notify.clone();

    let addr = format!("{}:{}", profile.hostname, profile.port);
    let mut session = client::connect(config, &addr, handler)
        .await
        .map_err(|e| AppError::Connection(e.to_string()))?;

    // Authenticate
    let authenticated = match profile.auth_method {
        AuthMethod::Password => {
            let pw = password
                .as_deref()
                .ok_or_else(|| AppError::Auth("Password required".into()))?;
            session
                .authenticate_password(&profile.username, pw)
                .await
                .map_err(|e| AppError::Auth(e.to_string()))?
        }
        AuthMethod::PublicKey => {
            let key_id = profile
                .key_pair_id
                .ok_or_else(|| AppError::Auth("No key pair selected".into()))?;
            let key_path = paths::private_key_path(&key_id);
            let key_pass = key_passphrase.as_deref().map(|s| s.as_str());
            let key_pair = russh_keys::load_secret_key(&key_path, key_pass)
                .map_err(|e| AppError::Auth(e.to_string()))?;
            session
                .authenticate_publickey(&profile.username, Arc::new(key_pair))
                .await
                .map_err(|e| AppError::Auth(e.to_string()))?
        }
        AuthMethod::Both => {
            let key_id = profile
                .key_pair_id
                .ok_or_else(|| AppError::Auth("No key pair selected".into()))?;
            let key_path = paths::private_key_path(&key_id);
            let key_pass = key_passphrase.as_deref().map(|s| s.as_str());
            let key_pair = russh_keys::load_secret_key(&key_path, key_pass)
                .map_err(|e| AppError::Auth(e.to_string()))?;
            let pk_ok = session
                .authenticate_publickey(&profile.username, Arc::new(key_pair))
                .await
                .map_err(|e| AppError::Auth(e.to_string()))?;

            if !pk_ok {
                let pw = password
                    .as_deref()
                    .ok_or_else(|| AppError::Auth("Password required for fallback".into()))?;
                session
                    .authenticate_password(&profile.username, pw)
                    .await
                    .map_err(|e| AppError::Auth(e.to_string()))?
            } else {
                true
            }
        }
    };

    if !authenticated {
        return Err(AppError::Auth("Authentication failed".into()));
    }

    let _ = event_tx.send(SshEvent::Connected).await;

    // Open a session channel with a PTY
    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| AppError::Connection(e.to_string()))?;

    channel
        .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .map_err(|e| AppError::Connection(e.to_string()))?;

    channel
        .request_shell(false)
        .await
        .map_err(|e| AppError::Connection(e.to_string()))?;

    // Start enabled tunnels
    let session_handle = Arc::new(Mutex::new(session));
    for tc in &profile.tunnels {
        if tc.enabled {
            tunnel::start_tunnel(session_handle.clone(), tc.clone(), event_tx.clone());
        }
    }

    // Main data loop
    let mut channel = channel;

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Ok(SshCommand::SendData(data)) => {
                        channel.data(&data[..]).await
                            .map_err(|e| AppError::Connection(e.to_string()))?;
                    }
                    Ok(SshCommand::Resize { cols, rows }) => {
                        channel.window_change(cols, rows, 0, 0).await
                            .map_err(|e| AppError::Connection(e.to_string()))?;
                    }
                    Ok(SshCommand::StartTunnel(tc)) => {
                        tunnel::start_tunnel(session_handle.clone(), tc, event_tx.clone());
                    }
                    Ok(SshCommand::StopTunnel(_id)) => {
                        // Tunnel stop is handled via drop of the tunnel task
                    }
                    Ok(SshCommand::Disconnect) | Err(_) => {
                        let _ = channel.eof().await;
                        let sess = session_handle.lock().await;
                        sess.disconnect(Disconnect::ByApplication, "User disconnected", "en")
                            .await
                            .map_err(|e| AppError::Connection(e.to_string()))?;
                        let _ = event_tx.send(SshEvent::Disconnected(None)).await;
                        return Ok(());
                    }
                }
            }
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        let _ = event_tx.send(SshEvent::Data(data.to_vec())).await;
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        log::info!("Remote process exited with status {exit_status}");
                    }
                    Some(ChannelMsg::Eof) | None => {
                        let _ = event_tx.send(SshEvent::Disconnected(None)).await;
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }
}
