//! Event Builder module for delila-rs
//!
//! チャンク＋ソート＋Safe Horizon 方式のイベントビルダー。
//! Legacy ELIFANT-Event と同じアプローチで、シンプルかつ高性能。
//!
//! # 主要コンポーネント
//!
//! - `SliceBuilder` - Time Slice 方式のオフラインイベント構築
//! - `L1Builder` - Moving Time Window 方式 (非推奨、互換性のため維持)
//! - `TimeCalibrator` - チャンネル間時間オフセット測定
//! - `chunk_builder` - チャンクベースのイベント構築 (v2, オンライン用)

mod built_event;
pub mod chunk_builder;
mod config;
mod hit;
mod l1_builder;
pub mod online;
pub mod pipeline;
mod root_io;
mod slice_builder;
pub mod source;
mod time_calibrator;
mod time_slice;
mod time_sort;

// Re-export main types
pub use built_event::{BuiltEvent, EventHit};
pub use config::{
    build_channel_map, get_trigger_channels, load_channel_config, load_l2_settings,
    save_channel_config, save_l2_settings, ChSettings, ChannelConfig, ConfigError,
    EventBuildingParams, L2LogicalOperator, L2Operator, L2Setting, L2Settings, TimeCalibration,
};
pub use hit::Hit;
pub use l1_builder::L1Builder;
pub use pipeline::{EventBuilderPipeline, PipelineConfig, PipelineStats};
pub use root_io::{
    read_hits_from_root, write_events_to_root, write_hits_to_root, write_time_histograms_to_root,
    RootError,
};
pub use slice_builder::{SliceBuilder, SliceBuilderStats};
pub use source::{DelilaFileHitSource, HitBatch, HitSource, SourceError};
pub use time_calibrator::{TimeCalibrator, TimeHistogram};

#[cfg(feature = "root")]
pub use source::RootFileHitSource;
