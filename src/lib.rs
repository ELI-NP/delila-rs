//! DELILA-RS: High-performance DAQ system for nuclear physics experiments
//!
//! This crate provides a distributed data acquisition pipeline using ZeroMQ.

pub mod common;
pub mod config;
pub mod data_sink;
pub mod data_source_emulator;
pub mod event_builder;
pub mod merger;
pub mod monitor;
pub mod node_agent;
pub mod offline;
pub mod operator;
pub mod reader;
pub mod recorder;
