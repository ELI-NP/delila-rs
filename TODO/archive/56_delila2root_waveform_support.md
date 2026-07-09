# TODO 56: delila2root 波形サポート + C++ 版退役

**Status: COMPLETED (2026-05-15)**
**Created:** 2026-05-15
**Completed:** 2026-05-15 (1 セッション)
**Hardware:** N/A (offline tool only)
**Plan file:** `~/.claude/plans/resilient-frolicking-boole.md`

## 検証結果 (2026-05-15)

### ローカル (Mac)
- `cargo fmt`: clean
- `cargo clippy --features root --bin delila2root --tests -- -D warnings`: 緑
- `cargo test --features root --bin delila2root`: 4/4 PASS
  - `push_event_with_waveform_populates_every_column`
  - `push_event_without_waveform_pads_with_defaults`
  - `mixed_events_keep_every_column_aligned`
  - `pre_amax_eventdata_via_serde_default_round_trips`
- `cargo build --release --features root --bin delila2root`: 緑

### gant 実機検証
- 元エラーが出ていた `~/tmp/data/run0001_0000_PHA2_Test.delila` (309 MB, 9585 events) を変換
- **9585 / 9585 events 完全変換** (元 C++ tool は "Events: 0/9585" でブロック parse 失敗)
- ROOT TTree 49 branch 全部出力確認 (1-indexed AnalogProbeType1..3, DigitalProbeType1..16, AnalogProbe1..3 vector, DigitalProbe1..16 vector, TimeResolution / TriggerThreshold / NsPerSample / AnalogProbe[1..3]IsSigned)
- 波形 spot-check (event 0):
  - `AnalogProbe1.size()=4096`, first 8 samples = `1536 1533 1533 1533 1538 1535 1541 1547` (PHA2 ADCInput probe ~1535)
  - `DigitalProbe1.size()=4096` (PHA2 Trigger probe)
  - `DigitalProbe16.size()=0` (PHA2 では未使用、empty Vec として正しく出力)
  - `TimeResolution=0`, `TriggerThreshold=100`, `NsPerSample=2.0`
  - `AnalogProbe2IsSigned=1` (TimeFilter probe = signed)
- hadd 圧縮: `hadd -f404 lz4.root pha2_test.root` で **308 MB → 58 MB (5.3x 圧縮)**
- ROOT で読めることも確認 (`delila->Show(0)` 正常表示)

### deployment
- gant の `/usr/local/bin/delila2root` (旧 C++ binary) は別途置き換え or 削除を推奨。 `/media/raid1/delila-rs/target/release/delila2root` をシンボリックリンクで参照する形がきれい

## ゴール

## ゴール

Rust 版 `src/bin/delila_to_root.rs` (binary 名は `delila2root` にリネーム) に波形ブランチを実装し、C++ 版 `tools/delila2root/` を完全退役する。

## 背景

ユーザーが gant 上で C++ 版 `tools/delila2root/delila2root` を使って PHA2 の `.delila` を ROOT に変換しようとしたところ次のエラー:

```
gant@gdedapp8:~/tmp$ delila2root -o runPha0001.root data/run0001_0000_PHA2_Test.delila
[1/1] data/run0001_0000_PHA2_Test.delila (9585 events, carry=0)        Warning: Failed to parse block 0 ...
  Events: 0 / 9585 in files
WARNING: Event count mismatch!
```

原因: `.delila` wire format が C++ tool 固定スキーマを越えて成長:

- **AMax 統合時** (`c07c11e`): `EventData` に `user_info: [u64; 4]` 追加 → array_size 6/7 (C++ 期待) → 7/8 (現行)
- **Phase 4.5** (`783c77e`): `Waveform` に `analog_probe_type[3]` / `digital_probe_type[16]` 追加
- **AMax debug FW** (TODO 55, 5/8): `analog_probe3` + `digital_probe5..16` + `is_signed × 3` 追加

C++ msgpack parser はポジション固定 (struct-as-array) なので毎回追従が必要。Rust 側は `#[serde(default)]` で前方/後方互換が自動。

既に Rust 版 `delila_to_root` (`e423692` Phase 3 commit) は scalar branches まで対応 (`Module/Channel/Energy/EnergyShort/TimestampNs/Flags/UserInfo0..3/HasWaveform/AnalogProbeType0..2/DigitalProbeType0..15`)。ただし doc-comment に `// Waveform data is intentionally skipped` で **波形は出力していない**。

ユーザー要件:
- 「波形は絶対に必要、デジタイザが返すなら必ず保存」
- 「全 probe 出す。empty Vec が並んでも圧縮で潰れる」
- 「圧縮は最速の (LZ4)」

## 設計判断

1. **Rust 拡張 + hadd 後処理**: oxyroot 0.1.25 (現行最新) は writer-side 圧縮非対応 ("can only write uncompressed file")。後処理で `hadd -f404 compressed.root uncompressed.root` (LZ4 level 4 = ROOT default fast) を案内。C++ tool 利用者は元々 ROOT を持っているので前提として OK。
2. **Binary リネーム** `delila_to_root` → `delila2root` (ユーザーが既にこの名前で叩いている、C++ tool との drop-in 置換)。ファイルパス `src/bin/delila_to_root.rs` は git history 維持のため変更しない。
3. **全 probe 出力** (analog 1..3 + digital 1..16): empty Vec が大量に並んでも uncompressed で ~22 B/event、圧縮後ほぼ 0。
4. **Branch 命名 1-indexed 統一**: 新規 vec branches は C++ tool 流の `AnalogProbe1`, `DigitalProbe1` 形式。**既存の `AnalogProbeType0..2` / `DigitalProbeType0..15` も `1..3` / `1..16` にリネーム**して統一。breaking change だが対象 binary は日が浅く影響軽微。
5. **波形なし event**: 全 vec → `Vec::new()`、metadata → 0/0.0/false。`HasWaveform` で gating。
6. **メモリモデル**: 現行どおり「全 event を Vec に貯めて `tree.write()` 一括」。oxyroot の API が `into_iter()` 前提なのでストリーミング化は大規模 refactor。doc-comment に目安 (5M events × 波形 = 数 GB RAM) 明記。
7. **C++ tool は完全削除** (Step 8 検証 PASS 後 `git rm -r tools/delila2root/`)。`legacy/` には移さない。

## Output branch list (final, 1-indexed throughout, total 49 branches)

**Scalar event branches** (既存 + 一部リネーム):

| Branch | Type | 状態 |
|---|---|---|
| Module, Channel | u8 | 既存 |
| TimestampNs | f64 | 既存 |
| Energy, EnergyShort | u16 | 既存 |
| Flags | u64 | 既存 |
| UserInfo0..3 | u64 (×4) | 既存 |
| HasWaveform | u8 | 既存 |
| **AnalogProbeType1..3** | u8 (×3) | **rename from `0..2`** |
| **DigitalProbeType1..16** | u8 (×16) | **rename from `0..15`** |

**新規 per-event waveform branches** (22 個):

| Branch | Type | 備考 |
|---|---|---|
| AnalogProbe1, 2, 3 | `Vec<i16>` (×3) | AP3 は AMax debug FW のみ非空 |
| DigitalProbe1..16 | `Vec<u8>` (×16) | DP1-4 PHA2/PSD2 標準、DP5 AMax debug、DP6-16 reserved |
| TimeResolution | u8 | 0 = 不明 |
| TriggerThreshold | u16 | wf なし→0 |
| NsPerSample | f64 | wf なし→0.0 |
| AnalogProbe1IsSigned, 2, 3 | bool (×3) | wf なし→false |

検証済: `oxyroot::WriterTree::new_branch` は `Iterator<Item = Vec<i16>>`, `Iterator<Item = Vec<u8>>`, `Iterator<Item = bool>` 全て対応 (oxyroot 0.1.25 tests/101__write_root_read.rs:313-410)。

## Implementation steps

### Step 0 — TODO ファイル作成 ✅ (本ファイル)
- TODO/CURRENT.md にも追加

### Step 1 — Cargo.toml リネーム
- L83 コメント更新、L231 `name = "delila2root"`

### Step 2 — collection loop 拡張
- accumulator 22 個追加、内側ループで全 column 対応、probe_type を 1-indexed リネーム

### Step 3 — write block 拡張
- `tree.new_branch(...)` 22 個追加、リネーム反映、hadd hint print

### Step 4 — Doc-comment header 更新
- 49-branch スキーマ、メモリ目安、hadd workflow

### Step 5 — Unit test 追加
- `#[cfg(test)] mod tests` で waveform round-trip

### Step 6 — Doc references 更新
- pha2_throughput_results.md / clean_architecture_evaluation.md / docs/plans/delila2root.md

### Step 7 — ローカル検証
```bash
cargo fmt
cargo clippy --features root --bin delila2root --tests -- -D warnings
cargo test --features root --bin delila2root
cargo build --release --features root --bin delila2root
```

### Step 8 — gant 実機検証
- rsync → rebuild → 失敗していた `~/tmp/data/run0001_0000_PHA2_Test.delila` を再変換 → **9585/9585 events** 確認
- hadd -f404 で圧縮 → サイズ比較
- 波形 spot-check (uproot or ROOT TBrowser)
- 余裕あれば pre-AMax `.delila` で backward compat 確認

### Step 9 — C++ tool 削除 + commit
- `git rm -r tools/delila2root/`
- このファイルを `**Status: COMPLETED**` に更新、検証ログ追記
- TODO/CURRENT.md から「最近完了」へ
- 1 commit: `feat(delila2root): waveform branches + retire C++ tool`

## 検証チェックリスト

- [ ] `cargo clippy --features root --bin delila2root --tests -- -D warnings` 緑
- [ ] `cargo test --features root --bin delila2root` 緑 (新規 unit test 含む)
- [ ] gant で 9585 / 9585 events 出力 (元エラー解消確認)
- [ ] `hadd -f404` で圧縮成功、サイズ ~1/3〜1/5
- [ ] ROOT で `AnalogProbe1` ブランチが空でない (spot check)
- [ ] (時間あれば) pre-AMax 古い `.delila` でも変換成功

## Risks / caveats

1. Probe-type branch リネームは breaking change (`AnalogProbeType0..2` → `1..3` 他)。現行 Rust 版利用者向け。doc + commit message に明記。
2. メモリ消費 (PHA2 5M events × full waveform ≈ 数 GB)。OOM 報告あれば streaming 版を別 task で。
3. uncompressed 出力サイズ (1 GB `.delila` → 数 GB ROOT)。hadd hint を目立つ print。
4. File-list ordering: argv 順処理、cross-file time sort なし (event_builder の責務)。
5. `[[bin]]` rename の波及: Cargo.toml + docs grep 済 (4 ヶ所のみ確認)。漏れあれば cargo build エラーで即発覚。

## 関連参照

- Plan ファイル: `~/.claude/plans/resilient-frolicking-boole.md`
- 既存実装: [src/bin/delila_to_root.rs](../src/bin/delila_to_root.rs)
- C++ tool (退役予定): [tools/delila2root/delila2root.cpp](../tools/delila2root/delila2root.cpp)
- 旧設計書: [docs/plans/delila2root.md](../docs/plans/delila2root.md)
- Phase 4.5 関連: [TODO/51_pha2_integration.md](51_pha2_integration.md)
- AMax 16-digital-probe 拡張: [TODO/55_amax_10g_fw_round2.md](55_amax_10g_fw_round2.md) (Phase H.2)
