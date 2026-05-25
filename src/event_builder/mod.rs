//! Event Builder module for delila-rs
//!
//! チャンク＋ソート＋Safe Horizon 方式のイベントビルダー。
//! Legacy ELIFANT-Event と同じアプローチで、シンプルかつ高性能。
//!
//! # 主要コンポーネント
//!
//! - `EventBuilderPipeline` (pipeline.rs) — オンライン/オフライン共通のコア
//!   (HitSource trait で入力差し替え。Sorter / Workers / Writers の std::thread 構成)
//! - `chunk_builder` — pure な event 構築ロジック (TriggerConfig + SortedChunk → BuiltEvent)
//! - `source` — `DelilaFileHitSource` / `RootFileHitSource` / `ZmqHitSource`
//! - `runtime_config` — `eb_config.json` (L1/L2 named-ops)
//! - `time_offsets` — `timeSettings.json` (tree モデル, DFS resolver)
//! - `SliceBuilder` — レガシー Time Slice (新パスは pipeline; テスト oracle として残置)
//! - `TimeCalibrator` — チャンネル間時間オフセット測定
//!
//! 旧 `online.rs` (711 行の独自パイプライン) は 2026-05-19 に削除。
//! Online EB は `bin/online_event_builder.rs` から `EventBuilderPipeline +
//! ZmqHitSource` を直接呼ぶ形に統一済み。

mod built_event;
pub mod chunk_builder;
mod config;
pub mod eb_message;
mod hit;
pub mod init;
pub mod l2_eval;
pub mod pipeline;
mod root_io;
pub mod runtime_config;
mod slice_builder;
pub mod source;
mod time_calibrator;
pub mod time_offsets;
mod time_slice;
mod time_sort;

// Re-export main types
pub use built_event::{BuiltEvent, EventHit};
pub use config::{
    build_channel_map, load_channel_config, load_l2_settings, save_channel_config,
    save_l2_settings, ChSettings, ChannelConfig, ConfigError, EventBuildingParams,
    L2LogicalOperator, L2Operator, L2Setting, L2Settings, TimeCalibration,
};
pub use eb_message::{BuiltEventBatch, EbMessage};
pub use hit::{Hit, HitLike, OfflineHit, OnlineHit};
pub use l2_eval::{ChannelTagMap, L2Filter, L2FilterError};
pub use pipeline::{EventBuilderPipeline, PipelineConfig, PipelineStats};
pub use root_io::{
    read_hits_from_root, write_events_to_root, write_hits_to_root, write_time_histograms_to_root,
    RootError,
};
pub use runtime_config::{
    ChannelRef, CmpOp, EbRuntimeConfig, L1Config, L1Op, L2Op, LogicOp, OutputConfig,
    RuntimeConfigError, TimingConfig,
};
pub use slice_builder::{SliceBuilder, SliceBuilderStats};
pub use source::{
    DelilaFileHitSource, HitBatch, HitSource, SourceError, ZmqHitSource, ZmqHitSourceError,
    ZmqHitSourceShutdown,
};
pub use time_calibrator::{TimeCalibrator, TimeHistogram};
pub use time_offsets::{
    ParentRef, ResolvedRow, ResolvedTimeOffsets, TimeOffsetEntry, TimeOffsetsError, TimeOffsetsFile,
};

#[cfg(feature = "root")]
pub use source::RootFileHitSource;
