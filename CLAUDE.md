# CLAUDE.md - DELILA-Rust (Next Gen DAQ)

## Absolute Rule — データ保全
**絶対にデータを落とさない。落とすくらいならシステムを止める。**
- ZMQ ソケットは全て HWM=0（無制限バッファ）。HWM をデフォルト(1000)に戻してはならない。
- チャンネル（tokio mpsc, crossbeam）は unbounded またはデータを落とさない設計にすること。
- バックプレッシャーでデータをドロップする設計は一切禁止。
- **明示的な例外（2026-07-09, TODO 58 C3/C4 決定）: Run Stop 直後のテールデータ**。Stop 時点でパイプライン（ZMQ/チャンネル）に残留していたバッチは Merger/Recorder が非 Running 状態で破棄する。定常データの末尾数秒の切断は統計的に無害と判断して受け入れ。ただし **silent にしない**: 破棄は `dropped_batches` に計上され Merger/Recorder のログに出る。**Running 中に dropped が増えるのは重大バグ**（Stop テールとの区別はログのタイミングで判別）。
- **明示的な例外（2026-07-13, TODO 58 L10/L11 明文化）**: ① Reader の Stop/シャットダウン時ドレインでの破棄（dig1 ドレイン上限 1000 件/1 s + CLEAR_DATA 含む）は上記 Stop テール例外の一部（warn 付き）。② **Monitor の bounded(1000) チャンネルドロップは表示専用パスとして恒久的に許容**（記録経路は Recorder が独立に持つ。ドロップはカウント済み）。この2つ以外の意図的ドロップを新設してはならない。

## Current Focus (2026-02)
**PSD1 ネットワーク透過テスト兼デコーダテスト中**
- Hardware: DT5730B (SN:990, DPP-PSD, USB) on Linux (172.18.4.147)
- Default config: `config/config_psd1_test.toml`
- 特に指示がなければこのテストを継続すること

## User Profile
- **User:** Aogaki - Senior Engineer, 27yr C++ experience, PhD in Computer Engineering
- **Role:** Claude = "Junior Rust Partner". Explain Rust via C++ analogies. Focus on ownership, memory layout, performance.
- Do not lecture on basic algorithms; focus on Rust-specific syntax and borrow checker resolutions.

## Project Overview
- **Goal:** MVP of distributed DAQ system by **mid-March 2026**
- **Hardware:** CAEN Digitizers (Optical Link/USB)
- **Architecture:** ZeroMQ pipeline: Reader → Merger → Recorder/Monitor
- **Reference:** C++ implementation in `legacy/DELILA2/` (local-only, gitignored)

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
  - **`--tests` を含めて clippy 通すこと**（テストコードでも `-D warnings` 維持）
- **Decoder hot-path で pattern-matching ヒューリスティック禁止**: spec page reference 必須 + `caen_simple_test` で実機検証必須。詳細は `src/reader/decoder/mod.rs` の "Hot-path heuristic policy" を参照（2026-05-04 PHA2 truncation 誤判定事案、commit `e641e99`）
- **Silent failure を作らない**: cache miss / 範囲外値 / FW 拒否は必ず `info!` 以上で可視化。debug! のまま埋めると数ヶ月後に発見される（2026-05-04 case-insensitive cache 事案、commit `e45e0ec`）

## Frontend Deployment Policy
- **`web/operator-ui/dist/` はリポジトリにコミット済み**。ユーザーは Rust のみでデプロイ可能、Node.js 不要
- UI (`web/operator-ui/src/`) を変更した開発者は `cd web/operator-ui && npm run build` → `dist/` も同じ commit に含める
- CI/pre-commit でチェックする場合は `git diff --exit-code dist/` で検証可能

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
- `docs/digitizer_system_spec.md` - デジタイザシステム仕様（DevTree, パラメーター等）
- `docs/compass_devtree_mapping.md` - CoMPASS↔DevTreeパラメーター対応表（全FW確定済）
- `docs/amax_fw_update_manual.md` - AMax FW 更新手順（`scripts/update_amax_fw.sh` 一発の codegen→build→UI→deploy）
- `docs/root_sink_manual.md` - root_sink（並列 ROOT recorder + JSROOT ライブモニタ）運用マニュアル
- `docs/devtree_examples/` - 実機から取得したDevTree JSON（パラメーター名の正確なリファレンス）
- `legacy/CoMPASS/` - CoMPASS設定画面スクリーンショット（UI設計のリファレンス）

## Benchmark Documentation
ベンチマーク結果は関連TODOファイルまたは設計ドキュメントに、測定日・条件・結果テーブル・結論を記録する。
