# TODO 65 — C++ ROOT シンク(スカラー ROOT Recorder + 簡易 Δt モニタ)

**Status: ✅ 実装+E2E 検証済(2026-07-21、`61c9a13`)— 残 = side3 デプロイ(ホスト復帰待ち)+ 実検出器での Δt 確認**

**検証結果(2026-07-21)**: ①単体 83/83(エンベロープ/デコード/マッチャ/ラン状態機械)
②Mac 手組みパブリッシャ E2E(リネーム・/Reset・シグナル)③gant ライブ AMax ストリーム受動購読
= reader カウンタ増分 385 と記録 385 が完全一致 ④gant ポート分離エミュレータスタックで
実 Rust 2 ラン E2E: run101=118,100 / run102=11,100 とも **Recorder .delila と完全一致**、
EOS ファイナライズ + `run%04d_scalar.root` リネーム + ラン間サイクル正動作。
gant は `~/.local/bin/root_sink` 配備済み。ビルドの no-sudo 変法(zmq.h 取得 +
libzmq.so.5 直リンク)は README 参照。
**発端:** side3/ThGEM テストで `.delila` → delila2root 変換の 2 段が冗長(スカラー 1 億 ev/run、
使うのは 5 フィールドのみ)。加えて ThGEM×2 + ガンマ線検出器の時間差をライブで見たい。

## 設計(2026-07-20 議論で確定)

**1 プロセス 2 役の汎用 C++ ツール**。merger の ZMQ PUB を**追加購読**する並列シンクで、
既存 Recorder(.delila 主記録)は無傷 — データ保全ルールと整合。**oxyroot は使わない**
(本家 ROOT ライブラリ TFile/TTree — 1000-entry basket バグ問題を構造的に回避)。

```
Reader → Merger ─PUB(tcp://*:5557)─→ Recorder (.delila 主記録、無傷)
                          ├────────→ Monitor(既存)
                          └────────→ ★root_sink (C++、本 TODO)
                                       ├→ run%04d_scalar.root (TTree "tr")
                                       └→ THttpServer :8090 (JSROOT ライブ表示)
```

### 受信・デコード

- libzmq(C API で十分)で SUB 接続。エンドポイントは config/CLI(side3 は
  `tcp://localhost:5557`)。HWM=0(データ保全ルール準拠。表示+便宜記録の複製経路であり
  主記録は Recorder が持つ)。
- ワイヤの `Message` エンベロープ(src/common/mod.rs:449: `Data(EventDataBatch)` /
  `EndOfStream{source_id, run_number}` / `Heartbeat`)を parse。**rmp-serde の enum
  エンコードの正確なバイト列は実装時に実メッセージを hexdump して確定**すること(小さい
  ラッパーなのでどちらの形式でも対応は数行)。
- `EventDataBatch` 本体は `.delila` ブロックと同一の rmp-serde compact MessagePack →
  **`tools/delila2root/TDelila.hpp` の `mp::Reader` + スキーマ処理を流用**(依存ゼロ
  ヘッダの再利用。ZMQ ストリーム用に薄いアダプタを足す)。

### Recorder 役

- フラット TTree `tr`(既存解析マクロと同じツリー名): `module/b, channel/b, energy/s,
  energy_short/s, timestamp_ns/D`。ZSTD(kDelilaCompression=505 を踏襲)。
- 見込みサイズ: raw 14 B/event → 100M ev で ~1.4GB、圧縮後 1GB 弱(現行変換出力 6.1GB
  → 大幅減 + 変換ステップ消滅)。
- **ラン境界は EOS 駆動**: EOS が `run_number` を運ぶ(TODO 58 C1 配線済み)。開始時は
  暫定名で書き、EOS で `run%04d_scalar.root` に確定リネーム + クローズ。operator への
  依存なし。次の Data 到着で新ファイル。
- 複数 source(将来の複数デジタイザ)混在でも module ブランチで区別可能。全 source の
  EOS が揃ったらクローズ(単一デジタイザ運用ではそのまま)。

### Monitor 役(簡易 Δt モニタ)

対象セットアップ: **単一 V1730** に ThGEM×2 + ガンマ線検出器。同一デジタイザなので
時間オフセット補正不要。

- 時間順に近い到着ストリームを小さなリングバッファに保持し、ガンマヒットごとに
  窓 ±W 内の ThGEM1/ThGEM2 ヒットを探索:
  - **TH1D `dt1`** = t(ThGEM1) − t(gamma)
  - **TH1D `dt2`** = t(ThGEM2) − t(gamma)
  - **TH2D `dt2_vs_dt1`**(Y=dt2, X=dt1)
- 表示は **THttpServer(JSROOT)**: `new THttpServer("http:8090")` + ヒスト登録のみ。
  ブラウザでライブ閲覧・ズーム・リセット。フロントエンド実装ゼロ。
- config(TOML or JSON、CLI 指定): gamma/ThGEM1/ThGEM2 のチャンネル番号、コインシデンス
  窓幅、ヒストのビン/レンジ、(将来用)エネルギーゲート、ZMQ エンドポイント、出力ディレクトリ、
  HTTP ポート。**チャンネル割当が config なので他実験(ELIADE 等)でも使い回し可**。

### 性能

ThGEM 実測 ~37 kHz はシングルスレッド C++ で余裕(参考: delila2root typed decode は
18.2M ev/37 s ≈ 500 kHz)。高レート時は TTree Fill と ZSTD が最初のボトルネック
→ 必要になったら IMT(`ROOT::EnableImplicitMT()`)を recorder 同様に有効化。

## 実装メモ

- 置き場所: `tools/root_sink/`(root_sink.cxx + README + 必要なら config 例)。
  ビルド: `g++ -O2 -std=c++17 root_sink.cxx $(root-config --cflags --libs) -lzmq`
  + `-lRHTTP`(THttpServer)。side3 に libzmq があるか要確認(無ければ user-local ビルド)。
- side3: ROOT=/opt/ROOT、バイナリは ~daq/.local/bin(delila2root と同じ流儀)。
- リポジトリは public(BSD-3)— 問題なし(CAEN 著作物・シミュレータ非含有)。
- 検証: ①エミュレータ or 実 DAQ で .delila と .root のイベント数/エネルギー総和一致
  ②意図的に既知遅延のパルサーで Δt ピーク位置確認 ③Stop→Start 連続でファイル境界と
  リネームの正しさ ④HWM=0 でのバックログ挙動(モニタ側は追いつき、記録は欠落ゼロ)。

## 完了条件

- [x] イベント数が Recorder(.delila)と一致(gant エミュレータ 2 ラン + ライブ AMax
      ストリームで検証。side3 実 DAQ での再確認はホスト復帰後)
- [x] EOS でのファイルクローズ/リネーム、複数ラン連続で正動作(run101/102 E2E)
- [x] THttpServer で dt1/dt2/2D がライブ更新(JSROOT JSON 配信確認、/Reset 動作確認)、
      窓・チャンネルは CLI フラグで設定可(チャンネル省略時は recorder 専用モード)
- [x] README(ビルド手順 no-sudo 変法込み + CLI + THttpServer の使い方)
- [x] TDelila.hpp は `#include "../delila2root/TDelila.hpp"` で共有(コピーなし)
- [ ] side3 復帰後: デプロイ + ThGEM 実検出器で Δt ピーク確認(既知遅延パルサー)
