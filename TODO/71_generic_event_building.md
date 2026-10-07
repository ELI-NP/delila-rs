# TODO 71 — root_sink 汎用イベントビルド(「まとめる層」)

**Status: 📋 OPEN (2026-10-07) — 設計合意済み、未着手**

**関連:** [event-builder/66_cpp_event_builder.md](event-builder/66_cpp_event_builder.md) Phase 2「汎用化」(本 TODO がその具体化) /
[70_root_sink_time_offsets.md](70_root_sink_time_offsets.md)(時刻オフセットが前提) /
[event-builder/SPECIFICATION.md](event-builder/SPECIFICATION.md) §6〜9 /
ELIFANT-Event(github.com/aogaki/ELIFANT-Event、ローカル `/Volumes/Ce140/ELIFANT-Event`) /
[72_digitizer_time_offset_calibration.md](72_digitizer_time_offset_calibration.md)

---

## 1. 発端と目的 (2026-10-07)

root_sink の出力は次の 2 つで、どちらも汎用のイベントではない。
- **`delila` ツリー:** ヒットを時刻順に並べただけ。
- **`built` ツリー:** ThGEM 専用。役割ごとに最も近い 1 本だけを持ち、x/y/TOF を計算済み。

ユーザーの目的は、**ニアオンラインモニタ**(オフラインでさっと解析して、データを取り続けるか判断する)に使える汎用のイベント出力。

イベントビルドは 2 層に分けて考える。
1. **まとめる層:** トリガーと窓でヒットを束ねる。どの実験でも同じ → **本 TODO で汎用に作る**。
2. **再構成する層:** ThGEM の XY/TOF、ELIADE の addback、ELIFANT の E-ΔE。実験ごとに違う → 実験を重ねてから共通化する。今の `built` は、ThGEM の再構成層の第 1 号。

時系列は `.delila` の役目(ユーザー指摘): `delila2root` でいつでも同じ形にできる。root_sink の hits ツリーは**標準では出さず、フラグで出せるようにだけ**する。

## 2. ELIFANT-Event の調査 (2026-10-07)

ELIFANT の実験で使われた汎用イベントビルダー(2025-05〜、34 コミット、コア約 3,000 行)。p91Zr の構成は 11 モジュール・208 ch(Si 128 / AC 32 / PMT 32 / HPGe 16)、トリガー 80 ch、AC 付き 24 ch。

| 層 | 中身 | 本 TODO での扱い |
|---|---|---|
| 時刻合わせ | トリガー ch 同士の時間差の分布のピークから、基準 ch に対するオフセットを求める(4 次元の表) | TODO 70(timeSettings)と TODO 72 が受け持つ |
| L1 | `IsEventTrigger` の ch を起点に、±窓内の全ヒット。出力は `TriggerTime` + `vector<RawData_t>`(mod, ch, chargeLong, chargeShort, 相対時刻, isWithAC) | **データモデルを採用**(ただし辞書不要の配列で出す) |
| 二重カウント | **ID 優先**: 窓内に ID の小さいトリガーがあればイベントを作らない(同じ ch は先が勝つ)。ヒットは重複しうる | 採用しない(§3、§4) |
| AC 判定 | ch ごとに AC の相手を指定し、窓内で鳴っていれば `isWithAC` を立てる(捨てずに印だけ) | Phase C で採用(ELIADE の clover + BGO に必要) |
| 閾値 | `ThresholdADC` 以下を、イベントを作る前に捨てる | 採用しない(SPEC v0.7: EB はエネルギーで切らない) |
| L2 | タグで数える(Counter) → 判定(Flag) → 採否(Accept: AND/OR)。数えた値と判定結果もブランチに出す | Phase C で移植 |
| チャンク境界 | エントリ数の重なりで処理(2026-09-25 にも境界での重複を修正) | root_sink の Sorter(データ内の時刻で区切る H10 方式)で置き換える |

## 3. 決定事項 (2026-10-07、ユーザー回答)

- **トリガー起点**(間隔で区切る方式ではない)。
- **出力は 1 エントリ 1 イベント、辞書不要の配列**(ELIFANT の vector + 辞書は使わない)。
- **ヒットは 2 つのイベントに重複させない。**
- **エネルギー閾値で捨てない。**
- **ELIFANT との互換は不要。** 用途はニアオンラインモニタ。**取りこぼしは無い方が断然よい。**
- **ELIFANT の解析は、Si を基準にすることに完全に依存している** → 基準は優先順位で選ぶ。
- ビームの仕様は未定(ELIADE は、当初は低エネルギー・低レートで重なりは少ない見込み)。ただし重なる場合も考慮する。

## 4. 規則

1. **トリガーの束ね方(取りこぼしなし)**
   - トリガーを時刻順に見る。まだ束ねていないトリガー t が来たら、[t, t+G] のトリガーを 1 つの束にする(**延長しない**)。
   - G より後のトリガーは、次の束の先頭になる。
   - → すべてのトリガーがちょうど 1 つのイベントに属する。ELIFANT の ID 優先で起きる連鎖での取りこぼし(A→B→C が 0.8W 間隔のとき、C が消える)は起きない。
2. **基準の選び方**
   - 束の中で、**設定で明示した優先順位**が一番高いトリガーを基準にする。同じ優先順位なら早い方。
   - 例: ELIFANT は Si E > Si ΔE > HPGe > PMT、ThGEM は ThGEM1 > ThGEM2。優先順位を指定しなければ、先頭のトリガーが基準。
3. **窓は基準を中心に取る:** [t_基準 − pre, t_基準 + post]。
   - 束の先頭を中心にすると、Si から見た窓の後ろ側が欠ける。基準を中心にすれば、基準からの相対時刻の範囲がいつも一定になる。
   - 束の中のトリガーがすべて窓に入るよう、**G ≤ pre** とする(読み込み時に検査)。対称な窓なら G = pre = post = W。
4. **ヒットは重複させない:** 2 つのイベントの窓に入るヒットは、**基準時刻が近い方**に入れる(同じ距離なら早い方)。窓の外のヒットは、どのイベントにも入らない(`.delila` には残る)。

**設定**(JSON、ラン開始ごとに読み直し、provenance を記録する仕組みは TODO 70 と共通)

```json
{"version": "1.0",
 "window_ns": {"pre": 1000, "post": 1000},
 "group_ns": 1000,
 "triggers": [
   {"name": "ThGEM1", "priority": 1, "channels": [[0, 1]]},
   {"name": "ThGEM2", "priority": 2, "channels": [[0, 6]]}]}
```

- ch が多い実験のために、`"tags": [...]` でも指定できるようにする(チャンネル表は Phase C)。
- **ThGEM の初期値:** 腕はトリガーの約 +20〜+260 ns 後、LaBr3 は約 70〜190 ns 前に来るので、±1000 ns で足りる。

**出力ツリー `events`**(辞書不要)
- イベント単位: `EventID/l`, `TriggerTime/D`(基準トリガーの補正後の絶対時刻), `TriggerMod/b`, `TriggerCh/b`, `TriggerIdx/b`(triggers[] の何番目か), `NTriggers/i`, `NHits/i`
- ヒット単位(時刻順): `HitMod[NHits]/b`, `HitCh[NHits]/b`, `HitEnergy[NHits]/s`, `HitEnergyShort[NHits]/s`, `HitTime[NHits]/D`(基準からの相対時刻), `HitIsTrigger[NHits]/O`
- Phase C で `HitWithAC[NHits]/O` を足す。

**監視(重なりへの備え)**
- 状態行とランのログに出す: 1 イベントあたりのトリガー数の分布、重なりで付け替えたヒット数、どの窓にも入らなかったヒット数。
- ビームが強くなって重なりが増えれば、ここで気づける。

## 5. 実装方針

- **規則 1〜2(逐次判定):** **Sorter(単一スレッド)でトリガーだけを先に処理**し、基準の一覧を決めてチャンクに添付する。トリガーは全ヒットのごく一部なので軽い。
  - 判定は「時刻 ≤ max_ts − safe_horizon」までのトリガーについて行い、チャンクの前後の文脈を pre+post+G 以上取る。
- **規則 3〜4(局所的な判定):** Worker で並列のまま、基準の一覧を見て各ヒットを振り分ける。
  - イベントは、**基準時刻がチャンクの core 範囲に入る**ものだけを出す(H10 と同じ)。
  - 出力は Worker の数に依存しない(既存の SeqReorder)。
- **Writer:** `events` ツリーを書く。hits ツリーは `--hits-tree` を付けたときだけ。`built` は当面残す。
- **設定と provenance:** `--event-def FILE` / TOML `event_def_file`。ラン開始ごとに読み直して固定し、失敗したら前の設定を維持して STALE にする。UserInfo と `_eventDef.json` に記録する。TODO 70 の仕組みを流用する。
- **時刻オフセット(TODO 70)はこの層より前で適用済み**なので、規則はすべて補正後の時間軸で動く。

## 6. 段階

- **Phase A — まとめる層:**
  - 規則 1〜4、配列での出力、監視用のカウンタ、設定 JSON の読み直し。
  - 単体テスト: 束が延長しないこと、A-B-C の連鎖で取りこぼしが無いこと、重なり時に近い基準へ振り分けること、優先順位による基準の選択、G ≤ pre の検査、非対称の窓、H10 相当の境界テスト(基準がチャンク境界をまたぐ場合)、Worker の数に依存しないこと。
  - E2E: run 56 の再生、ThGEM 設定で events を作る。
- **Phase B — ThGEM の `built` をこの層の上に作り直す:** run0023 の 4 本そろったイベント数 15,644 を再現できるかで検証(まとめる層の正しさの検証も兼ねる)。そのあと `built` の扱いを決める。
- **Phase C — ELIFANT からの移植(ELIADE の準備):** タグ付きのチャンネル表、AC の印(`HitWithAC`)、L2(Counter → Flag → Accept、出力に数えた値と判定結果)。
- **Phase D — マルチデジタイザ:** TODO 70 Phase 2(`M:C` 指定、`--module` の扱い)と TODO 72(ボード間の時刻合わせ)と合わせて行う。

## 7. 未決事項

- `HitTime` を D(8 B)にするか F(4 B)にするか(窓が µs なら F の精度でも十分)。
- `events` ツリーのファイル分割(今は 1 ラン 1 ファイル + ROOT の自動分割で対応)。
- JSROOT モニタ(XY、Δt)を、表示用のマッチャーのまま残すか、events から作るか。
- ELIADE の窓と優先順位(ビーム仕様の確定待ち)。
