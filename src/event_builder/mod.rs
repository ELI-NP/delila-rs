//! Event Builder module for delila-rs
//!
//! Time Slice 方式のイベントビルダー。
//! CBM/FLES で実績のある並列処理可能なアルゴリズムを採用。
//!
//! # アルゴリズム
//!
//! - **Time Slice**: 時間軸を固定サイズのスライスに分割
//! - **オーバーラップ**: 境界のイベントを正しく処理するための重複領域
//! - **並列処理**: rayon によるスライス単位の並列化
//!
//! # 主要コンポーネント
//!
//! - `SliceBuilder` - Time Slice 方式のイベント構築 (推奨)
//! - `L1Builder` - Moving Time Window 方式 (非推奨、互換性のため維持)
//! - `TimeCalibrator` - チャンネル間時間オフセット測定

mod built_event;
mod config;
mod hit;
mod l1_builder;
mod root_io;
mod slice_builder;
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
pub use root_io::{read_hits_from_root, write_events_to_root, write_hits_to_root, RootError};
pub use slice_builder::{SliceBuilder, SliceBuilderStats};
pub use time_calibrator::{TimeCalibrator, TimeHistogram};
pub use time_slice::{create_slices, TimeSlice};
pub use time_sort::TimeSortBuffer;
