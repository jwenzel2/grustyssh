use std::path::PathBuf;
use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zeroize::Zeroizing;

use crate::app::SshEvent;
use crate::error::AppError;
use crate::models::connection::ConnectionProfile;
use crate::ssh::session::establish_session;

#[derive(Debug)]
pub enum SftpCommand {
    ListDir(String),
    Upload { local: PathBuf, remote: String },
    Download { remote: String, local: PathBuf },
    MkDir(String),
    Remove(String),
    Rename { from: String, to: String },
    Disconnect,
}

#[derive(Debug, Clone)]
pub enum SftpEvent {
    Connected,
    DirListing { path: String, entries: Vec<SftpEntry> },
    TransferProgress { name: String, bytes: u64, total: u64 },
    TransferComplete { name: String },
    Error(String),
    Disconnected,
}

#[derive(Debug, Clone)]
pub struct SftpEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<u64>,
}

/// Spawn an SFTP session task. Returns the command sender.
pub fn spawn_sftp_session(
    profile: ConnectionProfile,
    password: Option<Zeroizing<String>>,
    key_passphrase: Option<Zeroizing<String>>,
    event_tx: async_channel::Sender<SftpEvent>,
) -> async_channel::Sender<SftpCommand> {
    let (cmd_tx, cmd_rx) = async_channel::bounded::<SftpCommand>(64);

    let rt = crate::runtime();
    rt.spawn(async move {
        if let Err(e) = run_sftp_session(profile, password, key_passphrase, event_tx.clone(), cmd_rx).await {
            let _ = event_tx.send(SftpEvent::Error(e.to_string())).await;
            let _ = event_tx.send(SftpEvent::Disconnected).await;
        }
    });

    cmd_tx
}

async fn run_sftp_session(
    profile: ConnectionProfile,
    password: Option<Zeroizing<String>>,
    key_passphrase: Option<Zeroizing<String>>,
    event_tx: async_channel::Sender<SftpEvent>,
    cmd_rx: async_channel::Receiver<SftpCommand>,
) -> Result<(), AppError> {
    // We need a separate event channel for the SSH layer (we ignore its events)
    let (ssh_event_tx, _ssh_event_rx) = async_channel::bounded::<SshEvent>(16);

    let session = establish_session(
        &profile,
        password.as_ref(),
        key_passphrase.as_ref(),
        ssh_event_tx,
    )
    .await?;

    // Open SFTP subsystem
    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| AppError::Connection(format!("Failed to open channel: {e}")))?;

    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| AppError::Connection(format!("Failed to request SFTP subsystem: {e}")))?;

    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| AppError::Connection(format!("Failed to initialize SFTP session: {e}")))?;

    let _ = event_tx.send(SftpEvent::Connected).await;

    // Command loop
    while let Ok(cmd) = cmd_rx.recv().await {
        match cmd {
            SftpCommand::ListDir(path) => {
                match sftp.read_dir(&path).await {
                    Ok(entries) => {
                        let mut listing = Vec::new();
                        for entry in entries {
                            let name = entry.file_name();
                            if name == "." || name == ".." {
                                continue;
                            }
                            let metadata = entry.metadata();
                            listing.push(SftpEntry {
                                name,
                                is_dir: metadata.is_dir(),
                                size: metadata.size.unwrap_or(0),
                                modified: metadata.mtime.map(|t| t as u64),
                            });
                        }
                        listing.sort_by(|a, b| {
                            b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                        });
                        let _ = event_tx.send(SftpEvent::DirListing { path, entries: listing }).await;
                    }
                    Err(e) => {
                        let _ = event_tx.send(SftpEvent::Error(format!("Failed to list {path}: {e}"))).await;
                    }
                }
            }
            SftpCommand::Upload { local, remote } => {
                let file_name = local.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                match tokio::fs::read(&local).await {
                    Ok(data) => {
                        let total = data.len() as u64;
                        let _ = event_tx.send(SftpEvent::TransferProgress {
                            name: file_name.clone(),
                            bytes: 0,
                            total,
                        }).await;

                        // Write in chunks for progress reporting
                        let remote_path = if remote.ends_with('/') {
                            format!("{}{}", remote, file_name)
                        } else {
                            remote.clone()
                        };

                        match sftp.open_with_flags(
                            &remote_path,
                            russh_sftp::protocol::OpenFlags::CREATE
                                | russh_sftp::protocol::OpenFlags::TRUNCATE
                                | russh_sftp::protocol::OpenFlags::WRITE,
                        ).await {
                            Ok(mut file) => {
                                let chunk_size = 32768;
                                let mut written = 0u64;
                                let mut error = None;

                                for chunk in data.chunks(chunk_size) {
                                    if let Err(e) = file.write_all(chunk).await {
                                        error = Some(e);
                                        break;
                                    }
                                    written += chunk.len() as u64;
                                    let _ = event_tx.send(SftpEvent::TransferProgress {
                                        name: file_name.clone(),
                                        bytes: written,
                                        total,
                                    }).await;
                                }

                                if let Some(e) = error {
                                    let _ = event_tx.send(SftpEvent::Error(
                                        format!("Upload failed for {file_name}: {e}")
                                    )).await;
                                } else {
                                    let _ = file.shutdown().await;
                                    let _ = event_tx.send(SftpEvent::TransferComplete {
                                        name: file_name,
                                    }).await;
                                }
                            }
                            Err(e) => {
                                let _ = event_tx.send(SftpEvent::Error(
                                    format!("Failed to open remote file {remote_path}: {e}")
                                )).await;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = event_tx.send(SftpEvent::Error(
                            format!("Failed to read local file {}: {e}", local.display())
                        )).await;
                    }
                }
            }
            SftpCommand::Download { remote, local } => {
                let file_name = remote.rsplit('/').next().unwrap_or(&remote).to_string();

                match sftp.open(&remote).await {
                    Ok(mut file) => {
                        // Get file size for progress
                        let total = sftp.metadata(&remote).await
                            .ok()
                            .and_then(|m| m.size)
                            .unwrap_or(0);

                        let _ = event_tx.send(SftpEvent::TransferProgress {
                            name: file_name.clone(),
                            bytes: 0,
                            total,
                        }).await;

                        let mut data = Vec::new();
                        match file.read_to_end(&mut data).await {
                            Ok(_) => {
                                let local_path = if local.is_dir() {
                                    local.join(&file_name)
                                } else {
                                    local.clone()
                                };

                                let _ = event_tx.send(SftpEvent::TransferProgress {
                                    name: file_name.clone(),
                                    bytes: data.len() as u64,
                                    total,
                                }).await;

                                match tokio::fs::write(&local_path, &data).await {
                                    Ok(_) => {
                                        let _ = event_tx.send(SftpEvent::TransferComplete {
                                            name: file_name,
                                        }).await;
                                    }
                                    Err(e) => {
                                        let _ = event_tx.send(SftpEvent::Error(
                                            format!("Failed to write {}: {e}", local_path.display())
                                        )).await;
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = event_tx.send(SftpEvent::Error(
                                    format!("Failed to read remote file {remote}: {e}")
                                )).await;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = event_tx.send(SftpEvent::Error(
                            format!("Failed to open remote file {remote}: {e}")
                        )).await;
                    }
                }
            }
            SftpCommand::MkDir(path) => {
                if let Err(e) = sftp.create_dir(&path).await {
                    let _ = event_tx.send(SftpEvent::Error(
                        format!("Failed to create directory {path}: {e}")
                    )).await;
                }
            }
            SftpCommand::Remove(path) => {
                // Try removing as file first, then as directory
                if let Err(_) = sftp.remove_file(&path).await {
                    if let Err(e) = sftp.remove_dir(&path).await {
                        let _ = event_tx.send(SftpEvent::Error(
                            format!("Failed to remove {path}: {e}")
                        )).await;
                    }
                }
            }
            SftpCommand::Rename { from, to } => {
                if let Err(e) = sftp.rename(&from, &to).await {
                    let _ = event_tx.send(SftpEvent::Error(
                        format!("Failed to rename {from} -> {to}: {e}")
                    )).await;
                }
            }
            SftpCommand::Disconnect => {
                let _ = event_tx.send(SftpEvent::Disconnected).await;
                return Ok(());
            }
        }
    }

    let _ = event_tx.send(SftpEvent::Disconnected).await;
    Ok(())
}
