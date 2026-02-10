use std::path::PathBuf;
use uuid::Uuid;

use crate::config;

pub fn private_key_path(id: &Uuid) -> PathBuf {
    config::keys_dir().join(format!("{}.key", id))
}

pub fn public_key_path(id: &Uuid) -> PathBuf {
    config::keys_dir().join(format!("{}.pub", id))
}
