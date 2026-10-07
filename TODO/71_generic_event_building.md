# TODO 71 — root_sink 汎用イベントビルド(「まとめる層」)

**Status: 🚧 Phase A・B・C 実装・検証済み (2026-10-07、コミット済み・未 push・pf-qaq 未配備) / 残 = ユーザー判断(`built` の扱い、Phase C の質問 11 個)、配備、Phase D の残り(TODO 70 Phase 2 / TODO 72)**

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
   - 束の中のトリガーがすべて窓に入るよう、**G ≤ min(pre, post)** とする(読み込み時に検査)。対称な窓なら G = pre = post = W。
     - (2026-10-07 実装時に訂正: 当初は「G ≤ pre」。基準が束の末尾なら他のトリガーは最大 G 前 → pre ≥ G、先頭なら最大 G 後 → post ≥ G が要る。)
4. **ヒットは重複させない:** 2 つのイベントの窓に入るヒットは、**基準時刻が近い方**に入れる(同じ距離なら早い方)。窓の外のヒットは、どのイベントにも入らない(`.delila` には残る)。
   - **トリガーのヒットは規則 1 の束に従う**(2026-10-07 実装時に明確化)。束の末尾のトリガーが、次の束の基準の方に近いことがある(例: 束 {0, 1000} の基準 0、次の束の基準 1001 → 1000 のトリガーは基準 1001 に 1 ns だが、束 0 のイベントに入る)。こうしないと「すべてのトリガーがちょうど 1 つのイベントに属する」が崩れる。

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

- **Phase A — まとめる層:** ✅ 2026-10-07 実装・検証済み(§8)
  - 規則 1〜4、配列での出力、監視用のカウンタ、設定 JSON の読み直し。
  - 単体テスト: 束が延長しないこと、A-B-C の連鎖で取りこぼしが無いこと、重なり時に近い基準へ振り分けること、優先順位による基準の選択、G ≤ pre の検査、非対称の窓、H10 相当の境界テスト(基準がチャンク境界をまたぐ場合)、Worker の数に依存しないこと。
  - E2E: run 56 の再生、ThGEM 設定で events を作る。
- **Phase B — ThGEM の `built` をこの層の上に作り直す:** ✅ 2026-10-07 再構成関数と検証まで済み(§9)。`built` の扱いはユーザー判断待ち run0023 の 4 本そろったイベント数 15,644 を再現できるかで検証(まとめる層の正しさの検証も兼ねる)。そのあと `built` の扱いを決める。
- **Phase C — ELIFANT からの移植(ELIADE の準備):** 調査と設計案 → [71_phase_c_proposal.md](71_phase_c_proposal.md)(2026-10-07、ユーザーへの質問 11 個つき)。✅ 2026-10-07 推奨案で実装・検証済み(§11)。質問への回答しだいで直す。 タグ付きのチャンネル表、AC の印(`HitWithAC`)、L2(Counter → Flag → Accept、出力に数えた値と判定結果)。
- **Phase D — マルチデジタイザ:** D1(ソフト側の `M:C` 対応、TODO 70 Phase 2)✅ 2026-10-07([TODO 70 の Phase 2 実装記録](70_root_sink_time_offsets.md))。汎用層そのものはモジュールをまたいで ¹⁴⁰Ce で検証済み(§10)。D2(ボード間の較正ツール、TODO 72)は進行中。 TODO 70 Phase 2(`M:C` 指定、`--module` の扱い)と TODO 72(ボード間の時刻合わせ)と合わせて行う。

## 7. 未決事項

- `HitTime` を D(8 B)にするか F(4 B)にするか(窓が µs なら F の精度でも十分)。
- `events` ツリーのファイル分割(今は 1 ラン 1 ファイル + ROOT の自動分割で対応)。
- JSROOT モニタ(XY、Δt)を、表示用のマッチャーのまま残すか、events から作るか。
- ELIADE の窓と優先順位(ビーム仕様の確定待ち)。

## 8. Phase A 実装記録 (2026-10-07)

### 何を作ったか

| ファイル | 中身 |
|---|---|
| `tools/root_sink/event_def.hpp`(新規) | 設定 JSON のパースと検証(全エラーをまとめて出す)、ラン開始時の読み直し(失敗したら前の設定を維持して STALE)、provenance JSON、ログ 1 行 |
| `tools/root_sink/eb_core.hpp` §7 | `TriggerGrouper`(規則 1〜2、Sorter で実行)、`build_generic_events`(規則 3〜4、Worker で実行)、`GenericStats`、`Sorter::set_lookback_ns` |
| `tools/root_sink/root_sink.cxx` | `--event-def` / `--hits-tree`、`events` ツリー、UserInfo `eventDef` + `_eventDef.json`、ランごとの集計行、状態行 `ev=N (cont= orph=)` |
| `tools/root_sink/check_events.C`(新規) | hits ツリーからイベントを 1 チャンクで作り直し、`events` ツリーと 1 件ずつ比べる(配備後の確認にも使う) |
| `config/event_def_thgem.json`(新規) | ThGEM 用: ±1000 ns、G = 1000 ns、ThGEM1 (p1, m0c1) > ThGEM2 (p2, m0c6) |
| `scripts/start_daq.sh` | TOML `event_def_file` → `--event-def`、`hits_tree = true` → `--hits-tree` |
| README / `docs/root_sink_manual.md` | 汎用イベントの節、hits ツリーの opt-in 化 |

### チャンク境界の扱い(設計の要点)

- Sorter は各チャンクの core 範囲のトリガーだけを順に `TriggerGrouper` に入れる。core_end より前のヒットは揃っている(Sorter の前提)ので、`開始 + G < core_end` の束は確定。
- 確定した境界 D = まだ開いている束の開始時刻(無ければ core_end)。あるヒットの振り分けには、そのヒットから +pre 以内の基準が全部確定している必要がある → このチャンクが出すのは、**基準時刻が [ev_lo, ev_hi) のイベント**(ev_hi = D − pre − post、ev_lo = 前のチャンクの ev_hi)。hits ツリー用の core 範囲とは別の範囲。
- そのため Sorter が前のチャンクから持ち越す範囲は **G + 2·pre + post**(ThGEM なら 4000 ns)。`--window-ns` と大きい方を使う。
- どの窓にも入らないヒットは、時刻が [ev_lo, ev_hi) に入るチャンクで数える → ラン全体で「イベント内のヒット + 窓外のヒット = core 範囲のヒット数」がちょうど成り立つ。Writer がラン終了時に検査し、合わなければ WARNING。

### 検証

- **単体テスト:** `test_eb_core` 256(うち汎用層 77)、`test_sink_core` 465(うち設定 JSON 158)。全合格、警告 0。
  - 規則 1〜4 を素直に書いた総当たりの参照実装と、ランダムな 200 通り(重なる束、時刻の同値、2 モジュール 3 種類のトリガー、非対称の窓、細かいチャンク、到着順の乱れ)で完全一致。
  - わざと 6 種類のバグを入れて検出できるか確認(持ち越し不足、ev_hi の余裕不足、2 つ目の窓の見落とし、束を閉じる条件の ≤、基準の削り過ぎ、同じ距離のとき遅い方)。束を閉じる条件だけ乱数では捕まらなかったので、束の終わりがちょうど core_end になる専用テストを足した。
- **E2E(Mac、`check_events.C` で 1 チャンクの作り直しと比較):**

| データ | 条件 | 結果 |
|---|---|---|
| run 56(pf-qaq、時刻オフセットあり) | workers 3 | IDENTICAL。33,463 イベント、イベント内 163,845 + 窓外 42,105 = 205,950 ヒット。hits / built は TODO 70 の結果と一致 |
| run0023(side3) | workers 1 / 8 / チャンク 0.2 ms | 3 通りとも IDENTICAL(40,045 イベント) |
| run0023 + LaBr3 を 3 番目のトリガーに(窓 −1500/+800、G 700) | workers 1 / 6 + チャンク 0.2 ms | IDENTICAL。764,911 イベント、うちトリガー 2 個が 7,177、取り合い 44 |
| 同上 | 1 バッチ 7 ヒット(約 13 万チャンク) | IDENTICAL |

- 起動時の検査・読み直しの失敗(STALE)・途中停止・ROOT の自動分割(パートごとに UserInfo)・配列バッファの拡張も確認済み(配線担当の E2E)。
- **データについての所見:** run 56 と run0023 では全イベントの NTriggers が 1。run 56 は ThGEM1/ThGEM2 がそれぞれ約 5 Hz で互いに独立(既存の built ツリーでも両面そろったイベントは 0、LaBr3 が付くのは 114)、run0023 は ThGEM2(ch6)のヒットが 0。汎用層の不具合ではない。
  - ユーザー確認 (2026-10-07): **今の ThGEM は線源を置いているだけなので、ThGEM 基準でイベントが重なることはほぼない**。重なり(NTriggers ≥ 2、取り合い)の実データでの確認はビームが来てから。それまでは単体テスト(総当たりとの一致)と、LaBr3 をトリガーにした負荷試験で担保する。

### pf-qaq への配備 — ✅ 2026-10-07 10:54 実施(3e6a4e9)

- ユーザーの指示(「配備やってください。start_daq.sh なども最新に」)。GitHub は使わず、Mac から ssh で pf-qaq の別ブランチに push → `git merge --ff-only`(0cfdeb9 → 3e6a4e9)。pf-qaq 側で手を入れた `config/digitizers/thgem_v1730_psd1_0.json`(UI 調整)と `monitor_layout.json` はそのまま残した。バックアップは `~/deploy_backup/todo71_20261007_105319/`。
- ステージング `~/tmp/todo71_deploy_3e6a4e9/` で GCC 13.3 / ROOT 6.38.06 のビルド(警告 0)と単体テスト 657 / 257 合格、別ポートで空起動を確認してから `~/.local/bin/root_sink` を入れ替え。戻し先は `root_sink.prev`(TODO 70 版)、その前は `root_sink.prev-pre-todo70`。
- 待機中(Configured)に `stop_daq.sh` → `start_daq.sh`。root_sink の引数は `... --time-calib ... --event-def config/event_def_thgem.json --hits-tree`(`--built-tree` 無し)、起動行に ERROR / WARNING 無し、JSROOT :8090 応答あり。Operator は Idle に戻った → **次のランの前に Configure が要る**。
- **残り:** 最初のラン(59)の後で `root -l -b -q "$HOME/delila-rs/tools/root_sink/check_events.C+(\"<run>.root\")"` が IDENTICAL になることを確認する。

### (配備前に書いた手順と注意)

**配備前テストは済み (2026-10-07、本番を止めずに `~/tmp/todo71_test/` で):** GCC 13.3 + ROOT 6.38.06 でビルド・単体テスト 465/256 合格。pf-qaq の run 57(生の時刻、288,571 ヒット)を別ポートで再生 → hits / built が本番と同数(built 46,544)、events 46,544(NTriggers すべて 1)、`check_events` と `compare_hits`(時刻オフセット込み)ともに IDENTICAL、`--hists` 付きの本番フラグでも同じ。`start_daq.sh` の TOML→引数の変換も模擬して確認。

**配備の罠(このテストで判明):** pf-qaq の `scripts/start_daq.sh` は古く、`hits_tree` / `event_def_file` を知らない → TOML にキーを足しても**黙って無視**され、新しいバイナリは hits ツリー無しで動く。**バイナリ・`start_daq.sh`・`config/event_def_thgem.json` を同時に入れる**こと。pf-qaq の TOML にはまだ TODO 71 の行が無いので、コメントを外すのではなく**キーを書き足す**。

1. 新しいバイナリと一緒に `scripts/start_daq.sh` と `config/event_def_thgem.json` を入れ、`config_thgem_pfqaq.toml` の `[network.root_sink]` に **`hits_tree = true`** を足す(足さないと hits ツリーが出なくなる。built だけになるので起動は通ってしまう)。`event_def_file = "config/event_def_thgem.json"` も足せば events ツリーが出る。
2. 入れ替えは TODO 70 §5 と同じ手順(ランとランの間に `root_sink.prev` を残して `install` → `mv`)。
3. 最初のランの後で `root -l -b -q 'check_events.C+("<run>.root")'` が IDENTICAL になることを確認する。

## 9. Phase B 実装記録 (2026-10-07)

### 何を作ったか

- `eb_core.hpp` §8 **`thgem_from_event(GenericEvent, BuilderConfig) → BuiltEvent`**: 汎用イベント 1 件から ThGEM の `built` 1 行を作る。突き合わせの規則は今の `build_events_from_chunk` とまったく同じ(面ごとに自分のトリガーから ±window で最も近い腕、同じ距離なら後ろ、LaBr3 は早い方のトリガーに最も近い 1 本)。
  - 規則を二重に持たないよう、面を埋める処理を `detail::fill_plane` / `fill_cross` に切り出し、今のビルダーもそれを使うように直した。今のビルダーの出力は、直す前のバイナリと `built` ツリーのハッシュが一致(run0023)。
- 変わるのは**候補の範囲だけ**: チャンク全体ではなく、そのイベントのヒットから選ぶ。だから 1 本のヒットが 2 行に使われることはない。
- `compare_built.C`(新規): `events` ツリーから `built` を作り直し、同じファイルの `built` ツリーと 1 行ずつ比べる。

### 今の `built` と一致する条件(単体テストで確認)

- 取り合いになるヒットが無い。
- イベントの窓が両側とも **group_ns + window_ns 以上**。ThGEM2 のトリガーは基準(ThGEM1)から最大 G 離れ、その腕はさらに window_ns 離れうるため。
- 作りの違い: 同じ面のトリガーが 1 つの束に 2 つ入ると、ここでは 1 行(早い方)、今のビルダーでは 2 行。

### 検証

| データ | 条件 | 結果 |
|---|---|---|
| 単体テスト | 疎なランダム 2,000 イベント(片面・両面・LaBr3・欠けた腕) | 今のビルダーと全行一致。`test_eb_core` 278 |
| run0023 | built window 500、イベント ±1000 / G 500 | **4 本そろい 15,644 を再現**、40,045 行すべて一致 |
| run 56 | 本番設定(built window 1000、イベント ±1000 / G 1000、時刻オフセットあり) | 33,463 行すべて一致(4 本そろい 14,660) |

### 決定 (2026-10-07、ユーザー): `built` ツリーはお役御免 → 案 3

- root_sink から `--built-tree` を削除(指定すると代わりの方法を示して exit 2)。TOML の `built_tree` は `start_daq.sh` が警告して無視する。
- チャンク単位のビルダー `build_events_from_chunk` を削除。ThGEM の再構成(`BuiltEvent`、`thgem_from_event`)は `thgem_reco.hpp` に移した。
- **`thgem_events.C`**(新規): `events` ツリーから、同じブランチの `built` ツリーを別ファイルに書き出す(UserInfo も写す)。run 56 と run0023 で、**以前の root_sink の built ツリーとハッシュが一致**(15,644 / 40,045 行、33,463 行)。`delayline_offsets.C` の `delayline_centres_built()` はその出力をそのまま読める。
- `compare_built.C` は比べる相手が無くなったので削除。

### (参考)検討した案

1. 今のまま(2 つのビルダーを並べて持つ)。
2. `--event-def` があるときは `built` を events から作る(Worker で `thgem_from_event`)。今のビルダーは後で削除。ヒットの二重使用がなくなる。ただし本番設定(±1000 / G 1000 / window 1000)は「窓 ≥ G + window」を満たさないので、両面が同時に鳴るビームでは ThGEM2 の腕が欠けうる → イベント窓を ±2000 にするか G を小さくする。
3. `built` ツリーをやめ、ThGEM の再構成はマクロで events から行う。

## 10. マルチデジタイザのビームデータでの負荷試験 (2026-10-07)

ThGEM は線源だけで重なりが無いので、ELIFANT p+¹⁴⁰Ce(2025、11 モジュール・208 ch)の生データで試した。

- **データ:** `/Volumes/Ce140/Raw/run0112_0000_p_140Ce.root`(ELIADE_Tree)を hits ツリーに変換(`timestamp_ns = FineTS/1000`)。**4,130 万ヒット / 35.9 s(約 1.15 MHz)**、時刻順に並んでいた。
- **時刻オフセット:** 本番 L1(`/Volumes/Ce140/Data/L1/production/run0112/timeSettings.json`)の TimeOffset はすべて 0 → 掛けずに再生。
- **設定 A(本番 L1 と同じトリガー):** Si ΔE リング 45 ch(m1〜m3、ELIFANT の IsEventTrigger)、優先順位なし、±2000 ns、G 2000。
- **設定 B(負荷用):** ΔE リング p1 > ΔE セクター p2 > HPGe p3 > LaBr3/CeBr3 p4、窓 −1500/+2500、G 1500、チャンク 1 ms。

| 設定 | 処理時間 | イベント | NTriggers ≥ 2 | 取り合い | `check_events` |
|---|---|---|---|---|---|
| A | 13 s(約 3.2 M ヒット/s) | 328,295 | 65,476(8 以上が 24,071) | 34,253 | **IDENTICAL**、イベント内 3,387,633 + 窓外 37,917,876 = 全ヒット |
| B | 32 s | 9,072,945 | 4,630,723 | 6,814,513 | **IDENTICAL**、イベント内 37,291,994 + 窓外 4,013,515 = 全ヒット |

- モジュールをまたぐトリガー・重なり・取り合いが大量にある実データでも、チャンク単位の結果が 1 チャンクの作り直しと完全に一致した。Phase D のうち「汎用層がモジュールをまたいで正しく動くか」は確認できた。残りは TODO 70 Phase 2(ThGEM の built / モニタが ch だけで引いている点)と TODO 72(ボード間の時刻合わせ)。
- 所見(ニアオンラインモニタとして): 設定 A では 2 µs の束にリングのトリガーが 8 本以上入るイベントが 24,071 ある(ノイズの多いリングか、多重度の高い反応)。こうした分布がランごとに状態行とログで見える。
- 変換マクロと設定は scratchpad に置いた(恒久化はしていない)。ログの 1 行に 45 ch が並んで読みにくかったので、連続する ch を `m1c0-14` とまとめるようにした。

## 11. Phase C 実装記録 (2026-10-07)

設計案([71_phase_c_proposal.md](71_phase_c_proposal.md))の**推奨案**で実装した。§2.7 の質問 11 個は未回答 — 回答が違えば該当部分だけ直す。

### 何を作ったか

- **設定 v1.1**(`event_def.hpp`、1.0 のファイルはそのまま読める):
  - `tags`(タグ → `[m, c]` / `[m, 最初, 最後]`)、トリガーの `tags`。
  - `anti_compton`: 窓は**印を付けるヒット自身から**の `[t − pre, t + post]`、`pairs` は detector / shield のタグ。
  - `l2`: counter(ヒット数を数える。`ac` = any / clean / vetoed)→ flag(6 種の比較)→ accept(AND / OR)。名前は C の識別子で、固定のブランチ名とは重ならない。
  - 空のリスト・知らない参照・名前の重複などはすべて読み込みエラー(まとめて出す)。
- **中核**(`eb_core.hpp`): 出すイベントに入れたヒットごとに AC を判定(チャンク全体の shield ヒットを二分探索、イベントとは無関係)。L2 はイベントごとに評価し、**印を付けるだけで捨てない**。
  - チャンクの約束: ev_hi をさらに ac_post だけ手前にし、持ち越しを G + 2·pre + post + ac_pre + ac_post に広げた。
- **出力**: `HitWithAC[NHits]/O` を常に出す(AC 無しの設定では全部 false)。L2 があるときは counter `名前/i`、flag / accept `名前/O`、`L2Accept/O`。ランのまとめ行に AC の印の数と L2 の集計。
- `check_events.C` は AC と L2 も作り直して比べる。

### 検証

| 項目 | 結果 |
|---|---|
| 単体テスト | `test_eb_core` 321(総当たりの参照実装に AC・L2 を足して、ランダム 200 通りで一致。わざと入れたバグ 5 種をすべて検出。うち「core_end より後に来る shield」は専用テストで捕まえた)、`test_sink_core` 657 |
| ThGEM run 56(v1.0 の設定のまま) | events の中身が Phase A と同じ(HitWithAC がすべて false で増えただけ、L2 ブランチ無し)。`check_events` IDENTICAL |
| ELIFANT p+¹⁴⁰Ce run 112(4,130 万ヒット) | ELIFANT の HasAC から 24 ペア(HPGe 4 + LaBr3/CeBr3 20、相手は同じ位置の BGO)、L2 は counter 6・flag 3・accept 2。`check_events` IDENTICAL(workers 6、および workers 1 + 1 ms チャンク = 21,099 チャンク)。依存しない別実装で HitWithAC を作り直しても不一致 0 |

- **実データでの AC:** Ge − BGO の Δt は幅の広いピーク(HPGe のタイミングウォーク、FWHM −78〜+124 ns)、LaBr3 − BGO は −82 ns に鋭いピーク(FWHM 29 ns)。窓を **[−300, +200] ns** にすると、偶発分を引いた真の同時計数の 98 % 以上が入る(窓内の偶発は約 4 %)。印が付いた割合は **HPGe 15.7 %、LaBr3/CeBr3 21.1 %**。
- **処理速度:** AC と L2 を足しても約 1 %(14.0 s → 14.2 s、4,130 万ヒット)。
- 140Ce 用の設定(`def_C.json`)と解析マクロは scratchpad にだけある(リポジトリには入れていない)。

### 既知の制限

- `TDelila.hpp` の JSON パーサーは、同じキーが 2 つあると後のものだけを残す → 同名のタグ 2 つなどは検出できない(共有ヘッダーの変更が要るので保留)。
- ThGEM の built と XY モニタは ch だけで引いている(モジュールを見ない)→ TODO 70 Phase 2。
