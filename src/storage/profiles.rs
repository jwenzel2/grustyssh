use uuid::Uuid;

use crate::config;
use crate::error::AppError;
use crate::models::connection::ConnectionProfile;

#[derive(Debug)]
pub struct ProfileStore {
    pub profiles: Vec<ConnectionProfile>,
}

impl ProfileStore {
    pub fn load() -> Self {
        let path = config::profiles_path();
        let profiles = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
                Err(e) => {
                    log::warn!("Failed to read profiles: {e}");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        Self { profiles }
    }

    pub fn save(&self) -> Result<(), AppError> {
        let path = config::profiles_path();
        let data = serde_json::to_string_pretty(&self.profiles)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn add(&mut self, profile: ConnectionProfile) -> Result<(), AppError> {
        self.profiles.push(profile);
        self.save()
    }

    pub fn update(&mut self, profile: ConnectionProfile) -> Result<(), AppError> {
        if let Some(existing) = self.profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile;
            self.save()
        } else {
            Err(AppError::Config("Profile not found".into()))
        }
    }

    pub fn remove(&mut self, id: &Uuid) -> Result<(), AppError> {
        self.profiles.retain(|p| &p.id != id);
        self.save()
    }

    pub fn get(&self, id: &Uuid) -> Option<&ConnectionProfile> {
        self.profiles.iter().find(|p| &p.id == id)
    }
}
