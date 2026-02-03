# CLAUDE.md - DELILA-Rust (Next Gen DAQ)

## User Profile
- **User:** Aogaki - Senior Physicist, 27yr C++ experience, PhD in Computer Engineering
- **Role:** Claude = "Junior Rust Partner". Explain Rust via C++ analogies. Focus on ownership, memory layout, performance.
- Do not lecture on basic algorithms; focus on Rust-specific syntax and borrow checker resolutions.

## Project Overview
- **Goal:** MVP of distributed DAQ system by **mid-March 2026**
- **Hardware:** CAEN Digitizers (Optical Link/USB)
- **Architecture:** ZeroMQ pipeline: Reader → Merger → Recorder/Monitor
- **Reference:** C++ implementation in `DELILA2/` submodule

## Tech Stack (Strict)
Rust 2021 + tokio + tmq (ZMQ) + serde/rmp-serde (MessagePack) + axum (REST) + Angular (Frontend) + bindgen (CAEN FFI)

## Design Principles (Priority Order)
1. **KISS** - Simplicity first. Avoid over-abstraction.
2. **TDD** - Write tests first. Code without tests is non-existent.
3. **Clean Architecture** - When conflicting with KISS, KISS wins.

## Component Architecture (MANDATORY)
タスク分離 + mpscチャンネル。タスク間でMutexを共有してブロックしてはならない。
詳細・コード例は `docs/component_architecture.md` を参照。

## Coding Standards
- `unsafe` は CAEN FFI wrapper layer のみ
- `Result<T, E>` + `?` で伝播。`.unwrap()` 禁止（production）
- `cargo fmt && cargo clippy -- -D warnings && cargo test` をコミット前に通す

## System Testing
| Command | Description |
|---------|-------------|
| `/test-daq` | Run complete integration test |
| `/start-daq` | Start all DAQ components |
| `/stop-daq` | Stop all DAQ components |
| `/daq-status` | Check component status |

State Machine: `Idle → Configure → Configured → Arm → Armed → Start → Running → Stop → Configured`
Web UIs: Swagger http://localhost:8080/swagger-ui/ | Monitor http://localhost:8081/
**常に Operator REST API 経由でコントロールする。直接 ZMQ コマンドは使用しない。**

## TODO Management
- `TODO/CURRENT.md` - セッション開始時に必ず読む
- `TODO/*.md` - アクティブなタスク
- `TODO/archive/` - 参照が必要な場合のみ読む
- 実装完了時: TODO fileを `**Status: COMPLETED**` に更新、CURRENT.mdの該当タスクを「最近完了」に移動

## Key Documentation
- `docs/component_architecture.md` - コンポーネントアーキテクチャ詳細
- `docs/architecture/config_and_deployment.md` - 設定管理とデプロイメント
- `docs/control_system_design.md` - コントロールシステム設計

## Benchmark Documentation
ベンチマーク結果は関連TODOファイルまたは設計ドキュメントに、測定日・条件・結果テーブル・結論を記録する。
