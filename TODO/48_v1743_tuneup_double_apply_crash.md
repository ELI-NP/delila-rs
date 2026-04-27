# V1743 Tune Up / Run Cycle Crash (libCAENDigitizer SIGSEGV)

**作成日:** 2026-04-23 / **最終更新:** 2026-04-27 — **解決 (真因確定)**
**ステータス:** ✅ RESOLVED — 真因は `MallocReadoutBuffer` を `apply_config_standard` の **前** に呼んでいたこと。35 KB / 35 MB の 1000× サイズ食い違いで CAEN DMA が user buffer を踏み越え → libCAENDigitizer.so 内部状態破壊 → SIGSEGV。修正は commit `45bb325`。
**関連:** [47_v1743_standard_mode_redesign.md](47_v1743_standard_mode_redesign.md), V1743 Settings UI 実装
**環境:** VX1743 SN:25 on 172.18.4.147, optical link (A3818 ではなく別カード), `config/config_x743_test.toml`

## ⚠️ 経緯: 4/24 fix は誤った真因 (根本ではなく症状緩和)

4/24 時点では「SIGTERM handler 未実装で stop_daq 時に CAEN handle リーク」を真因と認定し、SIGTERM handler + Drop 強化 + PLL poll + Tune Up 2 段 Apply skip を適用 (commits `2333c97`, `ed79d00`, `6751f53`)。

しかしこれは Cold Start (5 セッション) と Tune Up (3 セッション) しか検証しておらず、**いずれも各セッション間で reader プロセスが再起動する**。 4/27 に「**1 セッション内**で `/api/run/start` → `/api/stop` を繰り返す」strict test を実施したところ:
- 8/10 cycles でクラッシュ (cycle 9, libCAENDigitizer.so + 0x21224)
- Tune Up は 4/10 cycles でクラッシュ (cycle 5, +0x20de1)

→ 4/24 fix だけでは **session 内累積バグ** が残っていた。本日 (4/27) standalone reproducer (`x743_cycle_test --alloc-before-apply`) で T7 として真因を切り分け。

---

## 症状

Tune Up モードに入ると Reader プロセスが **`libCAENDigitizer.so` 内で SIGSEGV** して死ぬ。

UI 上の表示：
```
Apply error: Failed to restart pipeline after apply:
Group order=1 failed to reach Running:
Component errors: x743-vx1743: offline
```

Threshold 値が原因ではない（既知 good の 45874 でも crash）。**Tune Up 固有の問題**。

## Core dump 履歴（2026-04-23）

| 時刻 (EEST) | PID | 契機 |
|-------------|-----|------|
| 15:58:47 | 3778760 | （user 操作） |
| 16:01:20 | 3789256 | Tune Up Apply from UI（threshold 40000）|
| 16:19:22 | 3797813 | `/api/tuneup/start` 検証（threshold 45874）|
| 16:24:34 | 3801086 | `/api/tuneup/start` 再検証（threshold 45874）|

過去履歴: 4/22 に 3 回 (08:48, 11:38, 11:40)。**V1743 固有、PSD1/PSD2/PHA1 ではこの症状なし**。

全 crash の共通パターン：Reader log は `V1743 acquisition started` の直後で停止 → `SWStartAcquisition` から ~100 ms 後に SIGSEGV。

gdb 出力（一例）：
```
#0  0x00007f3143621224 in ?? () from /usr/lib/libCAENDigitizer.so
[stripped stack; all frames in libCAENDigitizer.so]
```

## 根本原因

[src/operator/routes/tuneup.rs:130-170](../src/operator/routes/tuneup.rs#L130-L170) の Tune Up Start が **2 段 Apply** を実行している：

1. `configure_all_sync` → Reader が 1 回目 `apply_config_standard(dc)`（Reset + 全パラメータ + 700 ms ADC calibration）
2. 直後（~80 ms 後）に `Command::ApplyDigitizerConfig(tuneup_config)` を送る → Reader が 2 回目 `apply_config_standard`（同じ処理、~2 秒）
3. `arm_all_sync` → Arm
4. `start_all_sync` → Start → `SWStartAcquisition` → **CAEN lib SIGSEGV**

2 回連続の `CAEN_DGTZ_Reset` + パラメータ設定 + ADC calibration シーケンスが libCAENDigitizer.so の内部状態を壊す。V1743 固有の lib バグ。

**Cold Start (`/api/configure → /api/arm → /api/start`) は 1 段 Apply で 問題なく動作**（本日 16:23 確認済み）。

## Tune Up の 2 段 Apply は X743Std では完全に冗長

[src/config/digitizer.rs:828-842](../src/config/digitizer.rs#L828-L842) の `force_software_trigger()` は X743 で **早期 return (no-op)**：

```rust
pub fn force_software_trigger(&mut self) {
    let sw_value = match self.firmware {
        FirmwareType::PSD1 | FirmwareType::PHA1 => "START_MODE_SW",
        FirmwareType::PSD2 | FirmwareType::AMax => "SWcmd",
        FirmwareType::X743CI | FirmwareType::X743Std => return, // ← no-op
    };
    ...
}
```

つまり X743Std の Tune Up Start では、configure_all_sync が適用する config と 2 回目 ApplyDigitizerConfig が送る config が **完全に同一**。2 回目の Apply はロジック上何も変更せず、CAEN lib を壊すだけ。

## 修正案

[tuneup.rs:146-200](../src/operator/routes/tuneup.rs#L146-L200) の「Pushing digitizer config to Reader (Tune Up)」ブロックに FW 判定を入れる：

```rust
// X743Std: configure_all_sync already applied identical config.
// Calling apply_config_standard() a second time within ~2s destabilizes
// libCAENDigitizer.so → SIGSEGV at SWStartAcquisition.
// force_software_trigger() is no-op for X743Std so the second apply is pure redundancy.
if let Some(config) = configs.get(&digitizer_id) {
    if config.firmware != FirmwareType::X743Std {
        // ... 既存の Pushing + force_software_trigger + ApplyDigitizerConfig ブロック
    } else {
        tracing::info!(digitizer_id, "X743Std: skipping redundant Tune Up Apply");
    }
}
```

同じ FW 判定は [tuneup.rs:409](../src/operator/routes/tuneup.rs#L409) 付近の `tuneup_apply` にも当てるべきか要検討（`tuneup_apply` 側は user が param を変更したとき呼ばれるので Apply 自体は必要。ただし**同じ config で Apply するケース** は skip できる）。

## 検証ステップ（再開時）

1. **コード修正**: `src/operator/routes/tuneup.rs` に上記 X743Std skip を追加
2. **ビルド**: `cargo build --release --features x743`
3. **デプロイ**: `rsync target/release/reader target/release/operator daq@172.18.4.147:~/delila-rs/target/release/`
4. **テスト**:
   - `./scripts/start_daq.sh config/config_x743_test.toml`
   - UI から Tune Up Start → Reader 生存確認
   - Settings タブで threshold 変更 → Apply → 再生存確認
   - 複数 Apply サイクル耐性確認（5-10 回）

## 想定される残課題

- **`tuneup_apply` 経路でも crash する可能性**: user が param 変更して Apply するたびに Reset + Apply が走る。これも short interval で重なると crash するかも → 最小間隔（例 3 秒）を強制するか、あるいは Apply 時に Reset を skip できるオプションを CAEN API で探す
- **CAEN lib 自体の脆さ**: 連続 Reset に弱い個体群。長期的には **Reader supervisor**（crash 時自動再起動）を入れておくのが安心。`start_daq.sh` に watchdog ループを追加 or systemd unit 化

## Settings UI（この日完成の別作業）

同じセッションで V1743 Settings UI を実装・デプロイ済：
- **変更**: [types.ts](../web/operator-ui/src/app/models/types.ts), [channel-params.ts](../web/operator-ui/src/app/models/channel-params.ts), [digitizer-settings.component.ts](../web/operator-ui/src/app/components/digitizer-settings/digitizer-settings.component.ts)
- **追加内容**: `FirmwareType` に `'X743Std'`, `X743Config` interface, Board/Input/Trigger/Energy/Coincidence(N/A)/Waveform タブの X743Std 分岐, `isGroupEnabled`/`toggleGroup` helper
- **デプロイ**: `web/operator-ui/dist/` を rsync で 172.18.4.147 に反映済
- **動作確認**: Board/Input/Trigger タブの表示は OK。ただし Apply ボタン経由の検証は crash でブロック → **本 TODO が解けた後に再確認必要**
- **残作業**（この修正で不要になる項目の検証）: Group Enable checkbox、Test Pulse、CFD 設定などの実操作確認

## 現状（2026-04-23 退勤時）

- Reader プロセス: **停止中**（最後の crash 以降、未再起動）
- Operator / Merger / Monitor / Recorder: 稼働中だが Reader 不在で Degraded 状態
- `config/digitizers/x743_test.json`: ch0 trigger_threshold = 45874 に復元済
- 再開時の最初の操作: `ssh daq@172.18.4.147` → `cd ~/delila-rs` → `./scripts/stop_daq.sh && ./scripts/start_daq.sh config/config_x743_test.toml`

---

## 2026-04-24 進捗: Primary Fix 適用

### 実施内容

1. **パッチ実装**: [src/operator/routes/tuneup.rs:146-170](../src/operator/routes/tuneup.rs#L146-L170) に `if config.firmware == FirmwareType::X743Std { skip }` を追加
2. **ビルド**: `cargo build --release --features x743` on 172.18.4.147 → 51 秒で完了
3. **デプロイ**: rsync src → リモートビルド（バイナリは `target/release/` に直接生成）
4. **動作検証**: 下記

### 検証結果

**Primary fix は動作 ✓**

DAQ を fresh boot した直後の **初回 Tune Up Session** で完全成功：
- `tuneup/start` (threshold=45874): Reader 生存 ✓
- 5 秒 run 安定: Reader 生存 ✓
- `tuneup/apply` (threshold=40000 に変更): Reader 生存 ✓ — **昨日 crash した値でも OK**
- `tuneup/apply` (threshold=45874 に戻す): Reader 生存 ✓

### Secondary Issue: 2 回目以降の Tune Up Start で crash

初回 Tune Up Session 終了後、**DAQ を再起動しても**次の `tuneup/start` で即 crash する：

| 試行 | シナリオ | 結果 |
|------|----------|------|
| #1 | fresh boot + tuneup/start | ✓ PASS（verify 4 phases） |
| #2 | stop_daq + start_daq + tuneup/start | ✗ Reader crash at SWStartAcquisition |
| #3 | stop_daq + start_daq + Cold Start（OK） + reset + tuneup/start | ✗ 同上 |
| #4 | stop_daq + start_daq + tuneup/start | ✗ 同上 |

Core dump pattern は昨日と同じ: `V1743 acquisition started` ログ直後、`libCAENDigitizer.so` 内で SIGSEGV。

**仮説**: V1743 ハードウェア（SAMLONG SRAM？）または光リンク PCIe カード駆動状態が、DAQ プロセスの stop/start を跨いで残る。初回の CAEN handle オープン直後は grace period で動くが、一度 `CAEN_DGTZ_Reset + apply + Arm + SWStartAcquisition + Stop + Reset` の 1 サイクルを通ると以降の SWStartAcquisition が crash する。

**試した範囲で DAQ スクリプト再起動だけでは復旧せず**。要 V1743 ハードウェア電源サイクル（物理的に）？ もしくは PCIe カードのカーネルドライバ再ロード？

### Core dump 履歴（2026-04-24）

- 07:52:02 — stress test run 1（verify が先に 1 Session 成功させた後）
- 07:54:41 — stress test run 2（fresh boot 後）
- 07:56:02 — verify test run 2（fresh boot 後）
- 07:57:58 — verify test run 3（cold start 成功後）

### 残課題 / 次の調査方針

1. **ハードウェア電源サイクル**: V1743 の電源を物理的に落として入れ直し、初回 Tune Up Session → Stop → 2 回目 Tune Up Session が通るか確認。通れば soft reset 手段を検討
2. **CAEN handle 強制再オープン**: Reader 側で Reset to Idle 時に CAEN handle を Close + Open（`handle: Option<Handle>` を None にして drop → 次 Configure で再 Open）。タイムスタンプ連続性は Tune Up モードでは不要なので OK。実装候補: [src/reader/mod.rs:2285-2294](../src/reader/mod.rs#L2285-L2294) あたり
3. **PCIe カードの kernel モジュール reload**: 関係ドライバ（`a2818`/`a3818` 系ではなく別カード）の rmmod + insmod で復旧するか
4. **Start 前の sleep 追加**: Arm と Start の間に N ms 挿入で CAEN 側の準備完了を待つ（`apply_config_standard` の ADC calibration が完全に収束していない疑い）

### 現状 (2026-04-24 08:00)

- Reader: 停止中（最新 crash 後）
- その他 4 プロセス (Operator/Merger/Monitor/Recorder): `./scripts/stop_daq.sh` で停止済
- JSON config: threshold 45874 (不変)
- Git 状態: `src/operator/routes/tuneup.rs` に patch 適用済・未 commit。`web/operator-ui/*` の昨日の Settings UI 変更も未 commit
- リモート `daq@172.18.4.147`: src 同期済、`target/release/` に新バイナリあり（reader/operator）

---

## 2026-04-24 午後: 真因特定 & 完全解決

### 診断プロセス

単独プロセスのテストバイナリ ([src/bin/x743_cycle_test.rs](../src/bin/x743_cycle_test.rs)) で本番 Reader の CAEN 呼び出しシーケンスを網羅的に模倣:

| テスト | Cycle | 結果 |
|--------|-------|------|
| Mode A/B/C/D (Stop 戦略 4 通り) | 20 | ✅ |
| Mode B + `--reapply` (毎 cycle 再 config) | 10 | ✅ |
| Mode B + `--reopen` (毎 cycle CAEN handle 再 Open) | 5 | ✅ |
| Mode D + `--fill-fifo` (Dirty Stop) | 10 | ✅ |
| Mode B + `--reapply --reopen --fill-fifo --no-guard` (完全本番模倣) | 10 | ✅ |

**結論: Start/Stop/Apply シーケンス自体は問題なし**。

次に `--reopen` を **プロセス SIGKILL** に置き換えたら Instance B の Open で `CommError` / `DigitizerNotFound` を再現。これが決定打。

### 真因

`pkill`（`stop_daq.sh` が使う）はデフォルト **SIGTERM**。Reader の main では `tokio::signal::ctrl_c()`（SIGINT のみ）しかハンドリングしていなかった → プロセスが Drop を走らせず即終了 → `CAEN_DGTZ_CloseDigitizer` 呼ばれず → CAEN カーネルドライバ側で handle が leak → 次の `OpenDigitizer` が最初 `CommError`、累積で `DigitizerNotFound`、最悪 `SWStartAcquisition` で segfault。

### 修正（4 点）

1. **Reader に SIGTERM handler 追加** ([src/bin/reader.rs](../src/bin/reader.rs))
   ```rust
   use tokio::signal::unix::{signal, SignalKind};
   let mut sigterm = signal(SignalKind::terminate())?;
   tokio::select! {
       _ = tokio::signal::ctrl_c() => {},
       _ = sigterm.recv() => {},
   }
   // shutdown broadcast → Drop runs → CloseDigitizer called
   ```

2. **`X743Handle::Drop` を WaveDemo 準拠に強化** ([src/reader/caen_legacy/handle.rs:824-855](../src/reader/caen_legacy/handle.rs#L824-L855))
   - 従来: CloseDigitizer のみ
   - 修正後: SWStopAcquisition → ClearData → CloseDigitizer の 3 段

3. **Board Fail Status (0x8178) poll で PLL lock 確認**
   - `wait_for_board_ready`: bit 4 (PLL lock loss) がクリアされるまで 50ms 間隔で poll、5s でタイムアウト
   - `apply_config_standard` 末尾と `sw_start_acquisition` 直前で呼出
   - Reset 直後の PLL 再ロックを正しく待つ（CAEN lib は待たずに SWStart すると segfault）

4. **Tune Up の 2 段 Apply skip** ([src/operator/routes/tuneup.rs:146-226](../src/operator/routes/tuneup.rs#L146-L226))
   - X743Std では `force_software_trigger` が no-op なので `configure_all_sync` 済み config と同一
   - 冗長な 2 回目の `apply_config_standard` (Reset + 全 register + ADC cal ~2s) を skip

### 検証結果

すべて `daq@172.18.4.147` 実機 (VX1743 SN:25)：

- **Cold Start cycle (stop_daq/start_daq x5)**: 5/5 PASS、Reader ALIVE 全 cycle
- **Tune Up multi-session (start_daq → Tune Up Start+Apply x2 → Tune Up Stop → stop_daq、3 セッション)**: 3/3 PASS
- 新規 core dump 0 件

### 教訓

- `tokio::signal::ctrl_c()` は **SIGINT のみ** 。SIGTERM は別途 `unix::signal(SignalKind::terminate())` で拾う必要
- pkill 系でプロセスを kill する系のスクリプトを持つコードでは全 component binary で SIGTERM handler 必須
- CAEN ドライバは handle 未 Close を厳格にトラッキング、リークすると以降の Open 系 API が壊れる
- V1743 の PLL は Reset 後に一時的にアンロックする、bit 4 (0x10) が 0 になるまで poll 待機が正しい
- テストバイナリで isolation 実験するのは有効だが、multi-process / signal handling の問題は isolation では見えない → プロセス境界の振る舞い（kill signal 含む）も明示的にテストする

### Git 状態（2026-04-24 11:00）

- `src/bin/reader.rs` + `src/reader/caen_legacy/handle.rs` + `src/operator/routes/tuneup.rs` に修正、未 commit
- `src/bin/x743_cycle_test.rs` + `src/bin/x743_stop.rs` 新規追加（デバッグ用、残しておく価値あり）
- 昨日の Settings UI 変更（`web/operator-ui/*`）も未 commit
- リモート `daq@172.18.4.147`: 最新バイナリデプロイ済

---

## 参考

- Settings UI 設計相談ログ: このセッションの最初の方
- 既存 Tune Up 実装: [archive/28_tuneup_mode.md](archive/28_tuneup_mode.md)
- x743 Phase 1 (FFI 接続): [45_v1743_support.md](45_v1743_support.md)
- x743 Standard mode 再設計: [47_v1743_standard_mode_redesign.md](47_v1743_standard_mode_redesign.md)

---

## 2026-04-27 真因確定 (FINAL RESOLUTION)

### きっかけ

「Tune up mode や普通の Run で Configure が走る形での Start/Stop を繰り返す」strict test をユーザから依頼。これまでの 4/24 fix は **stop_daq → start_daq でセッション再起動** の検証しかしていなかった。session 内で連続サイクルすると未だ crash する → 4/24 fix は症状緩和に留まっていたことが判明。

### Test battery (`x743_cycle_test` に flag 群を追加)

1 セッション内で `--reapply` × 30 cycles をベースラインに、production との差分を 1 つずつ standalone に追加して再現を狙う:

| Test | Flag | 仮説 | 結果 |
|------|------|------|------|
| T1 | `--decode-events` | DecodeEvent が壊す (H-Decode) | PASS 30/30 |
| T2 | `--realloc-buf` | buffer realloc が必要 (H-Buf, sanity) | PASS 30/30 |
| T3 | `--double-reset` | mod.rs の eager Reset + apply 内 Reset 連発 (P2) | PASS at cycle 19 |
| T6 | `--zmq-noise` (kitchen sink) | tokio + ZMQ scheduling (P3) | PASS 30/30 |
| **T7** | `--alloc-before-apply` | **`MallocReadoutBuffer` を apply 前に呼ぶ (H-Buf direct)** | **CRASH every cycle** |

### 真因

`CAEN_DGTZ_MallocReadoutBuffer` は **現在の board 設定** に基づいて buffer サイズを決定する (UM1935 p.22)。production の旧コード ([reader/mod.rs:2148](../src/reader/mod.rs#L2148)) は Open 直後 (default state) に malloc していた:

| Alloc タイミング | `allocated_size` | ReadData 結果 |
|---|---|---|
| Pre-apply (production の旧バグ) | **35,856 bytes** | `CAEN_DGTZ_OutOfMemory`, 0 events |
| Post-apply (修正後) | **35,840,016 bytes** (約 1000×) | events 正常取得 |

サイズ不一致 → CAEN background DMA thread が user buffer を踏み越え → libCAENDigitizer.so 内部ポインタ破壊 → 数サイクル後の SWStartAcquisition で SIGSEGV (offset `+0x21224` / `+0x20de1`)。

### 修正 commits

1. **`8f6ce55`** `fix(operator/run_start)`: Phase 1.5 X743Std skip (= 4/24 fix の `tuneup_start` 修正と対称)。Cycle 1 の double-Apply による即死を防ぐ
2. **`45bb325`** `fix(reader/v1743)`: **真因 fix**。`MallocReadoutBuffer` を `apply_config_standard` 成功後に移動。Configure / ApplyConfig / ApplyConfigRunning すべてで再 alloc
3. **`838dfbb`** `feat(bin/x743_cycle_test)`: T1–T7 isolation flags を追加 (将来同種バグの再現用)
4. **`d976daf`** `fix(reader/v1743)`: マニュアル監査による 4 fix
   - WaveDemo 準拠で channel self-trigger を全 disabled してから enable
   - SW trigger を全 mode で ACQ_ONLY 維持
   - SWStartAcquisition 直前の ClearData 削除
   - LoadSAMCorrectionData の conditional 化 (Reset 後 SAMCorrectionDataLoaded をチェック)

### Hardware 検証 (172.18.4.147 VX1743 SN:25, 10 kHz pulser)

- 修正前: Run cycle 9 / Tune Up cycle 5 で SIGSEGV
- 修正後: **30/30 Run cycles + 5/5 Tune Up cycles PASS**, 各 ~67k events, 0 core dumps, reader 生存
- Apply latency 修正前後で同等 (~2.06 s)
- Reset 後も `SAMCorrectionDataLoaded=true` を確認 → conditional LoadSAM が常にスキップ動作

### 教訓 (memory に追加済み)

- [caen_buffer_alloc_order.md](../../.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/caen_buffer_alloc_order.md) — 「`MallocReadoutBuffer` は configure 後に呼ぶ。`SetRecordLength` 等の後に再 alloc 必須」
- [v1743_tuneup_double_apply_crash.md](../../.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/v1743_tuneup_double_apply_crash.md) — 真因の最終版 (4/24 partial fix と 4/27 真因 fix の経緯)

### Session 跨ぎ vs Session 内テストの教訓

「Cold Start 5x / Tune Up 3x セッション全 PASS」は**セッション間** (各 stop_daq + start_daq で reader プロセス再起動) のテスト。**1 セッション内で連続サイクル** すると別バグが残ることを示した良い例。今後 V1743 系の修正検証では必ず両方のテストを行うべき。

### FELib (PSD2/PSD1/PHA1) では同種バグなし

確認: FELib (`caen_lib2`) は `CAEN_FELib_ReadData` に **Rust 側 `Vec<u8>`** ([reader/mod.rs:1257](../src/reader/mod.rs#L1257)) を渡すだけ。`MallocReadoutBuffer` 相当の lib-side allocator は無い。Buffer ownership が Rust 側なのでサイズ不一致による DMA overflow は構造的に起きない。本件は `caen_legacy` (libCAENDigitizer 直接) を使う digitizer ファミリ (V1743/V1740/...) 固有。
