//! Event Builder Config Repository - MongoDB storage for Event Builder settings
//!
//! Stores chSettings, timeSettings, and L2Settings with version history.
//! Compatible with ELIFANT-Event JSON formats.

use chrono::{DateTime, Utc};
use mongodb::{
    bson::{doc, oid::ObjectId},
    Client, Collection,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

use crate::event_builder::{ChannelConfig, L2Settings, TimeCalibration};

/// Event Builder configuration document stored in MongoDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBuilderConfigDocument {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,

    /// Configuration name (user-defined)
    pub name: String,

    /// Experiment name (for grouping)
    pub exp_name: String,

    /// Version number (incremented on each save)
    pub version: u32,

    /// When this config was created/updated
    pub created_at: DateTime<Utc>,

    /// Who/what created this config
    pub created_by: String,

    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Whether this is the current active config
    pub is_current: bool,

    /// Channel settings (chSettings.json)
    pub ch_settings: ChannelConfig,

    /// Time calibration (timeSettings.json)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_settings: Option<TimeCalibration>,

    /// L2 filter settings (L2Settings.json)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l2_settings: Option<L2Settings>,

    /// Event building parameters
    #[serde(default)]
    pub coincidence_window_ns: f64,

    /// Slice duration for Time Slice method [ns]
    #[serde(default = "default_slice_duration")]
    pub slice_duration_ns: f64,
}

fn default_slice_duration() -> f64 {
    10_000_000.0 // 10 ms
}

/// Repository errors
#[derive(Error, Debug)]
pub enum EventBuilderRepoError {
    #[error("MongoDB error: {0}")]
    Mongo(#[from] mongodb::error::Error),

    #[error("Config not found: {0}")]
    NotFound(String),

    #[error("Configuration error: {0}")]
    Config(String),
}

/// MongoDB repository for Event Builder configurations
#[derive(Clone)]
pub struct EventBuilderRepository {
    configs: Collection<EventBuilderConfigDocument>,
}

impl EventBuilderRepository {
    /// Create a new repository using an existing MongoDB client
    pub fn new(client: &Client, database: &str) -> Self {
        let db = client.database(database);
        Self {
            configs: db.collection::<EventBuilderConfigDocument>("event_builder_configs"),
        }
    }

    /// Save an Event Builder configuration (creates new version)
    pub async fn save_config(
        &self,
        mut config: EventBuilderConfigDocument,
        created_by: &str,
    ) -> Result<EventBuilderConfigDocument, EventBuilderRepoError> {
        let name = config.name.clone();
        let exp_name = config.exp_name.clone();

        // Get current version number
        let current = self.get_current_config(&name, &exp_name).await?;
        let next_version = current.map(|c| c.version + 1).unwrap_or(1);

        // Mark existing current as non-current
        self.configs
            .update_many(
                doc! { "name": &name, "exp_name": &exp_name, "is_current": true },
                doc! { "$set": { "is_current": false } },
            )
            .await?;

        // Update config fields
        config.id = None;
        config.version = next_version;
        config.created_at = Utc::now();
        config.created_by = created_by.to_string();
        config.is_current = true;

        self.configs.insert_one(&config).await?;

        info!(
            name = name,
            exp_name = exp_name,
            version = next_version,
            "Saved Event Builder config"
        );

        Ok(config)
    }

    /// Get the current (active) configuration
    pub async fn get_current_config(
        &self,
        name: &str,
        exp_name: &str,
    ) -> Result<Option<EventBuilderConfigDocument>, EventBuilderRepoError> {
        let doc = self
            .configs
            .find_one(doc! { "name": name, "exp_name": exp_name, "is_current": true })
            .await?;
        Ok(doc)
    }

    /// List all current configurations for an experiment
    pub async fn list_configs(
        &self,
        exp_name: &str,
    ) -> Result<Vec<EventBuilderConfigDocument>, EventBuilderRepoError> {
        use futures::TryStreamExt;

        let cursor = self
            .configs
            .find(doc! { "exp_name": exp_name, "is_current": true })
            .sort(doc! { "name": 1 })
            .await?;

        let configs: Vec<EventBuilderConfigDocument> = cursor.try_collect().await?;
        Ok(configs)
    }

    /// List all experiments that have configurations
    pub async fn list_experiments(&self) -> Result<Vec<String>, EventBuilderRepoError> {
        let experiments = self
            .configs
            .distinct("exp_name", doc! { "is_current": true })
            .await?;

        let names: Vec<String> = experiments
            .into_iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        Ok(names)
    }

    /// Get version history for a configuration
    pub async fn get_config_history(
        &self,
        name: &str,
        exp_name: &str,
        limit: i64,
    ) -> Result<Vec<EventBuilderConfigDocument>, EventBuilderRepoError> {
        use futures::TryStreamExt;

        let cursor = self
            .configs
            .find(doc! { "name": name, "exp_name": exp_name })
            .sort(doc! { "version": -1 })
            .limit(limit)
            .await?;

        let configs: Vec<EventBuilderConfigDocument> = cursor.try_collect().await?;
        Ok(configs)
    }

    /// Get a specific version
    pub async fn get_config_version(
        &self,
        name: &str,
        exp_name: &str,
        version: u32,
    ) -> Result<Option<EventBuilderConfigDocument>, EventBuilderRepoError> {
        let doc = self
            .configs
            .find_one(doc! { "name": name, "exp_name": exp_name, "version": version })
            .await?;
        Ok(doc)
    }

    /// Restore a specific version as the current config
    pub async fn restore_version(
        &self,
        name: &str,
        exp_name: &str,
        version: u32,
    ) -> Result<EventBuilderConfigDocument, EventBuilderRepoError> {
        let old_config = self
            .get_config_version(name, exp_name, version)
            .await?
            .ok_or_else(|| {
                EventBuilderRepoError::NotFound(format!("{}:{} v{}", exp_name, name, version))
            })?;

        // Save as new version with description
        let mut restored = old_config.clone();
        restored.description = Some(format!("Restored from version {}", version));

        self.save_config(restored, "restore").await
    }

    /// Delete a configuration (all versions)
    pub async fn delete_config(
        &self,
        name: &str,
        exp_name: &str,
    ) -> Result<u64, EventBuilderRepoError> {
        let result = self
            .configs
            .delete_many(doc! { "name": name, "exp_name": exp_name })
            .await?;

        info!(
            name = name,
            exp_name = exp_name,
            deleted = result.deleted_count,
            "Deleted Event Builder config"
        );

        Ok(result.deleted_count)
    }

    /// Update only ch_settings for a config
    pub async fn update_ch_settings(
        &self,
        name: &str,
        exp_name: &str,
        ch_settings: ChannelConfig,
        created_by: &str,
    ) -> Result<EventBuilderConfigDocument, EventBuilderRepoError> {
        let mut config = self
            .get_current_config(name, exp_name)
            .await?
            .ok_or_else(|| EventBuilderRepoError::NotFound(format!("{}:{}", exp_name, name)))?;

        config.ch_settings = ch_settings;
        config.description = Some("Updated chSettings".to_string());

        self.save_config(config, created_by).await
    }

    /// Update only time_settings for a config
    pub async fn update_time_settings(
        &self,
        name: &str,
        exp_name: &str,
        time_settings: TimeCalibration,
        created_by: &str,
    ) -> Result<EventBuilderConfigDocument, EventBuilderRepoError> {
        let mut config = self
            .get_current_config(name, exp_name)
            .await?
            .ok_or_else(|| EventBuilderRepoError::NotFound(format!("{}:{}", exp_name, name)))?;

        config.time_settings = Some(time_settings);
        config.description = Some("Updated timeSettings".to_string());

        self.save_config(config, created_by).await
    }

    /// Update only l2_settings for a config
    pub async fn update_l2_settings(
        &self,
        name: &str,
        exp_name: &str,
        l2_settings: L2Settings,
        created_by: &str,
    ) -> Result<EventBuilderConfigDocument, EventBuilderRepoError> {
        let mut config = self
            .get_current_config(name, exp_name)
            .await?
            .ok_or_else(|| EventBuilderRepoError::NotFound(format!("{}:{}", exp_name, name)))?;

        config.l2_settings = Some(l2_settings);
        config.description = Some("Updated L2Settings".to_string());

        self.save_config(config, created_by).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_builder::ChSettings;

    fn create_test_ch_settings() -> ChannelConfig {
        vec![vec![ChSettings {
            id: 0,
            module: 0,
            channel: 0,
            is_event_trigger: true,
            threshold_adc: 100,
            has_ac: false,
            ac_module: 128,
            ac_channel: 128,
            detector_type: "HPGe".to_string(),
            tags: vec!["detector".to_string()],
            p0: 0.0,
            p1: 1.0,
            p2: 0.0,
            p3: 0.0,
        }]]
    }

    #[test]
    fn test_config_document_serialization() {
        let doc = EventBuilderConfigDocument {
            id: None,
            name: "default".to_string(),
            exp_name: "p91Zr".to_string(),
            version: 1,
            created_at: Utc::now(),
            created_by: "test".to_string(),
            description: Some("Test config".to_string()),
            is_current: true,
            ch_settings: create_test_ch_settings(),
            time_settings: None,
            l2_settings: None,
            coincidence_window_ns: 500.0,
            slice_duration_ns: 10_000_000.0,
        };

        let json = serde_json::to_string(&doc).unwrap();
        assert!(json.contains("\"name\":\"default\""));
        assert!(json.contains("\"exp_name\":\"p91Zr\""));
        assert!(json.contains("\"version\":1"));
    }
}
