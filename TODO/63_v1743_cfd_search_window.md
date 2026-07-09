# TODO 63 — V1743 CFD 探索窓が遅い立ち上がりパルスで交差を取り逃す

**Status: 📋 OPEN (2026-07-09 発見、TODO 62 作業中に判明)**

## 症状

x743 の CFD テスト2件が `cfd_valid == false` で fail:
- `test_x743_cfd_negative_pulse_sub_sample_timing`
- `test_x743_cfd_positive_pulse_finds_edge`

**TODO 62 とは無関係。** git blame で確認: このテストと `X743WaveformStats::analyze` の探索窓ロジックは**同じコミット `e4ad305`（2026-04-23, Standard mode 導入）で同時に生まれ、以来ずっと fail**。x743 テストは CAEN が要り CI で回らないため、誰にも気づかれずにいた test⇔impl の食い違い。

## 根本原因

[analyze()](../src/reader/mod.rs) の CFD 後方探索窓が短すぎ、遅い立ち上がりパルスのリーディングエッジ・ゼロ交差が窓の外に落ちる:

```rust
let search_span = (cfd_delay * 4).max(16);   // cfd_delay=4 → 16 samples
```
コメントも「rise time < 4·delay を仮定」と明記。しかしテストのパルスは:
- negative: rise = **32 samples**（= 8·delay）
- positive: rise = **24 samples**（= 6·delay）

負パルスの具体:
- baseline[0..64]=0, rise[64..96]=0→−1000, hold[96..128]=−1000
- `peak_index` = 最初の −1000 = **sample 95**（`min_by` は最初の最小を返す）
- CFD ゼロ交差（リーディングエッジ）= **sample ~65.7**
- 探索窓 = `[95−16, 95]` = **[79, 95]** → 交差点は窓外 → 交差見つからず → `cfd_valid=false`

正パルスも同型（peak_index=87, 交差~50, 窓[71,87] で外れ。`max_by` は最後の最大を返すので flat-top 末尾）。

**寄与要因:** `peak_index` が flat-top の遠端に来る（min_by=先頭 tie=95 / max_by=末尾 tie=87）ため、探索開始点がリーディングエッジからさらに遠のく。

## 影響（実機）

- 遅い立ち上がりパルスでは CFD が無言で peak fallback（サンプル量子化タイミング）に落ちる。既知の CFD 限界 [[v1743_energy_known_limitation]] と同根。
- `cfd_delay` を立ち上がりに合わせて設定すれば `rise ≈ 4·delay` に収まり analyze() は正常に交差を取る。→ **実機の V1743 パルス形状と cfd_delay 設定次第**で、テストのパルスが非現実的なだけの可能性もある。

## 修正オプション（要判断）

1. **探索窓を広げる/スケール**: `search_span` を `peak_index - n_bl`（立ち上がり全域）ベースに上限付きで拡大。遅い立ち上がりにも robust になる。ただし長い pre-trigger 窓でノイズ交差を拾うリスクとのトレードオフ（元々窓を絞った理由がこれ）。
2. **テストを現実的に**: 実機の代表的 rise/delay 比に合わせてテストのパルスを作り直す（実機データ or CoMPASS 波形を参照）。
3. **両方**: analyze() を rise に追従させ、テストも実機準拠にする。

**+ silent failure 対策**: `cfd_valid=false` → peak fallback は無言。CLAUDE.md「silent failure を作らない」に沿って、fallback 発生を rate-limited `warn!`/カウンタで可視化すべき（何ヶ月も気づかれなかった今回の事案そのもの）。

## 着手前に

実機（172.18.4.147 SN25 / host_side3）で 10kHz パルサーの実波形の立ち上がり時間を測り、`cfd_delay` 設定と突き合わせて「analyze() が悪いのか test が非現実的なのか」を確定してから修正方針を選ぶこと。推測で窓を広げない。

## 関連

- [[v1743_energy_known_limitation]] — CFD/energy は simple amplitude のまま
- [[v1743_first_pulse_2026-04-22]] — 実機クリーンパルス取得済
