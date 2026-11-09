# TODO 73 — 2026-11 実験キャンペーン(pf-qaq: Si + 中性子 → ThGEM + 中性子)

**Status: 🚧 N_Wall 立ち上げ中 (§5) — 2026-11-05 に pf-qaq を 6 台の構成へ切り替え、Configure 成功(今は全ボード独立スタート)。残 = 最初のランと初日の確認 §5.4、クロック結線後の同期**

**関連:** [74_grafana_event_rate_monitor.md](74_grafana_event_rate_monitor.md) /
[75_mesytec_hv_monitor.md](75_mesytec_hv_monitor.md) /
[76_caen_hv_crate_monitor.md](76_caen_hv_crate_monitor.md) /
[config/config_thgem_pfqaq.toml](../config/config_thgem_pfqaq.toml) /
`docs/pfqaq_operations_overview.pdf`(運用の概要、2026-10-07 時点)

---

## 1. 計画(ユーザー、2026-10-22)

| 時期 | 実験 | 検出器 |
|---|---|---|
| **11 月中旬** | 1 本目、`experiment_name` = **`N_Wall`** | **Si + 中性子(液シン 24 台)** のみ |
| その後 | 2 本目以降 | **ThGEM + 中性子** + 細かいもの |

**Si(2026-10-22 回答):** 今はチャンネル 1 つだけ(**module 0 = V1730 SN66 の ch 1**)。11 月中旬までに
**S1 か S7 を 2 枚**にする。読み出しは別に持ってくる **V1725(DPP-PHA、16 ch)× 8 台 = 128 ch**(当初「6 台」と回答 → 同日に 8 台と訂正)


**Si のチャンネル(同):** E と dE の 2 層で、**それぞれ リング 15 × 3(inner / middle / outer)+ セクター 16 = 61 ch**、
**計 122 ch** → V1725 × 8 = 128 ch に収まる(空き 6 ch)。

**時間相関(同):** **Si と中性子の同時計測は要る。** V1725 × 6 と VX2730 のクロック・スタートの結線と設定は
ユーザーが行う。

**root_sink(同):** ThGEM は使わないので、`N_Wall` では**別の root_sink**(ThGEM 用の設定は使わない)。
いずれ本格的なイベントビルダーにしたいが、今は後回し(土台は TODO 71 の汎用層)。

**実験名(同):** `N_Wall` で進める(変わることもある)。

**HV(同):** Si = **Mesytec MHV-4 が 1 台**、中性子 = **CAEN SY5527**(IP 不明のため後回し)。

準備はほぼ終わっている(ユーザー)。残りの柱は次の 2 つ。

- **Grafana をイベントレートモニタとしてちゃんと使う** → [TODO 74](74_grafana_event_rate_monitor.md)
- **HV の読み出し**: Si は Mesytec のモジュール → [TODO 75](75_mesytec_hv_monitor.md)、
  中性子は CAEN の HV クレート → [TODO 76](76_caen_hv_crate_monitor.md)。
  どちらも InfluxDB に書いて Grafana に出す

## 2. 準備状況(2026-10-22 時点)

| 項目 | 状態 |
|---|---|
| DAQ PC | pf-qaq(daq@192.168.144.142、DHCP)。`/data`(NVMe)+ `/archive`(ZFS)+ `archive_sync.sh` 稼働中 ✅ |
| 中性子(液シン 24 台) | VX2730 DPP-PSD = source 1(`PSD2-VX2730-LS`)。eno1 直結(pf-qaq 172.18.4.50/22 ↔ 2730 172.18.4.57)、10-20 に Configure 成功(47 パラメータ、0 エラー)。値は液シン向けの初期値のまま(未チューニング)、32 ch 全部有効。**未コミット** |
| Si | 今は 1 ch(module 0 ch 1)。11 月中旬に **V1725 PHA × 8 台(128 ch、使うのは 122 ch)**で読む。設定ファイル・結線は未着手 |
| ThGEM | V1730 SN66 = source 0、root_sink(`module = 0`)、時刻オフセット(TODO 70) |
| Grafana | データは来ているが、長い期間が表示できない・ダッシュボードが Fission2026 向けのまま → TODO 74 |
| HV(Si / 中性子) | 未実装 → TODO 75 / 76 |
| Monitor の 2D Save Image | 修正して pf-qaq に配備済み(10-20)。**未コミット** |
| リポジトリ | **未 push のコミット 24 個 + 未コミットの作業**(2730 設定、UI 修正、pha_fw_scan、ポスター、運用 PDF)。実験前に片付ける |

## 3. 実験ごとの DAQ 設定(案)

1 本目(Si + 中性子)と 2 本目(ThGEM + 中性子)で、使うボード・root_sink・`experiment_name` が変わる。
`config.toml` のシンボリックリンクを切り替える運用(SN01/SN02 と同じ)にして、TOML を実験ごとに分ける案:

- `config/config_nwall_pfqaq.toml`(1 本目、`experiment_name = "N_Wall"`): VX2730 + V1730 + V1725 PHA × 8 の 10 台(§3.1)。
  root_sink は ThGEM 用とは別の設定(当面はヒットの記録とライブヒストグラム程度、本格 EB は後回し)

### 3.1 `N_Wall` の DAQ 構成 v1(2026-10-29、ユーザーと決定)

> **2026-10-30 変更(ユーザー):** dE = リング + Sector、**E = Sector のみ**(E のリング 3 台はなし)→ V1725 は 5 台。
> 液シン・γ・ビームパルスを VX2730 にまとめる案あり(TOF を 1 台で閉じる)。詳細は [TODO 79](79_root_sink_nwall_online.md) §5。

**10 台構成。VX2730 がマスター、ほかはスレーブ。** source / module の番号は案(変えてよい)。

| module(案) | ボード | 入力 | 空き ch | 接続 |
|---|---|---|---|---|
| 0 | VX2730 DPP-PSD(マスター) | 中性子ウォールの液シン 24 台**だけ** | 8(パルサー用に 1 本) | eno1 直結(172.18.4.57) |
| 1 | V1730 DPP-PSD(今の ThGEM のボード、SN66) | LaBr3 × 7 + **ビームパルス 1/100(ch15)** | 8(パルサー用に 1 本) | A3818 光リンク |
| 2 | V1725 DPP-PHA | dE の Sector 16 | 0 | A3818 光リンク(1 リンク 2 台ずつ) |
| 3〜5 | V1725 DPP-PHA × 3 | dE のリング(inner / middle / outer、各 15 → ch0〜14) | 各 1(**ch15 = パルサー**) | 同上 |
| 6 | V1725 DPP-PHA | E の Sector 16 | 0 | 同上 |
| 7〜9 | V1725 DPP-PHA × 3 | E のリング(各 15 → ch0〜14) | 各 1(**ch15 = パルサー**) | 同上 |

- **慣習: パルサーなどのタイミング用の信号は最後の ch(ch15)に入れる**(ユーザー)。
- V1725 は `type = "pha1"`、`time_step_ns = 4.0`。V1730 は `type = "psd1"`、`time_step_ns = 2.0`。
- **クロックとスタート:** デジタイザ間のハードのカスケード。PLL の設定はユーザー。スタートは一発で入るので、
  ボード間の**オフセットはマスターとスレーブの間で最大 20 ns 程度(一定)、ランごとの揺れは 4 ns 程度**(ユーザー)。
- **スタートディレイ:** N_Wall のケーブルが非常に長いので、各ボードのスタートディレイ(DIG1 `start_delay`、DIG2 は対応する
  パラメーター)で時間差を最小にする。**初めて本格的に使うので、ケーブル分も含めてパルサーで測る**(TODO 72 の `time_map.C`
  がずれ → スタートディレイの変更案 → パルサーでの確認まで出す)。
- **ランごとの揺れ(4 ns)の追跡:** パルサーを常時入れられるボード(リング用 6 台の ch15、VX2730、V1730)はパルサーで、
  空きのない Sector 用の 2 台は、同じ粒子が鳴らすリングと Sector の時刻差からデータで測る。液シンは γ フラッシュの
  TOF ピークでも t0 を確認できる。
- **対応表:** 1 本にまとめる(ユーザー賛成)。Grafana の書式 `module channel name group` の 5 列目以降に、層(dE / E)・
  リング / Sector の番号などを足し、Grafana・root_sink・オフライン解析が同じ表を読む(TODO 79)。
- **eLog:** N_Wall 用のログブックを ThGEM とは別に作る(ユーザー)。
- **オンライン表示(root_sink、Monitor は簡単なものだけ):** 中性子の TOF、dE リング × E リングの相関、dE Sector × E Sector
  の相関(TODO 79)。
- **Operator の復帰の道:** ボードが S-IN を受けられないときなどに、`stop_daq.sh` で殺さずに戻れるようにする(TODO 78)。

### 3.2 時間相関の確認(DAQ 側でやること)

結線と設定はユーザー。DAQ 側では次を用意・確認する。**本番は VX2730 がマスター**(全ボードを S-IN で同期スタート、ユーザー 2026-10-29)。
スタート手順のコード確認と実機テストの計画・結果は [TODO 77](77_multiboard_start_sync.md)(本番構成は今のコードで対応済み)。

1. スタートの揃え方: DIG1 の V1725 同士(S-IN / 光リンク経由)と DIG2 の VX2730 の両方が、同じ合図で
   タイムスタンプを 0 にするか。過去の記録では、DIG1 の S-IN はレベルトリガーで、ボード間に約 650 ms の
   ずれが出た例がある(Operator の `arm_all_sync` で HW の Armed を待つようにした経緯あり)。
   **実例 (2026-10-22 17:08、pf-qaq):** VX2730(mod 1)を S-IN でスタートする設定にしたが、V1730(mod 0)が
   スタート信号を出さない設定のままだったため、Start が完了せず繰り返しになった(ユーザーが DAQ を立ち上げ直して解決)。
   ログ上は V1730 が Running を報告しないまま Operator が約 5 秒で諦める形。**S-IN でスタートを受けるボードを作ったら、
   信号を出す側(マスター)の出力設定も必ず合わせる。** Grafana の `run_event` には失敗した試行も start / stop の組として残る。
2. ボード間の時刻オフセット: パルサーを全ボードに入れたランで `tools/root_sink/time_map.C`(TODO 72)を回し、
   `timeSettings.json`(TODO 70)を作る。
3. V1725 で波形を録る場合は record length に注意(V1725 は 7168 ns 以上でカップル内の同時イベントが片方消える。
   DIG1 のカップル アービターの件)。
- `config/config_thgem_pfqaq.toml`(2 本目): 今のもの + VX2730

切り替えは Operator の再起動が必要(設定は起動時に読む)。ラン番号は `experiment_name` ごとに続く。

### 3.3 ビームパルスとのコインシデンス(検討中、2026-10-29)

- 目的: **dE/E テレスコープとビームパルスのコインシデンス**(オフライン解析のトリガーとして使う。ゲートや VETO ではない)。
- dE と E はそれぞれ別のデジタイザで、どちらも 16 Sector(+ リング 45)。**ビームパルスは 5 MHz** で全部は記録できない。
- 候補:
  1. **ダウンスケールして記録 + オフラインで位相を復元**(推奨): ビームが高周波に同期した厳密な周期なら、間引いたパルスから
     周期と位相を当てはめ、全イベント(Si も液シンの TOF も)に直近のバンチ時刻を付けられる。前提 = ボード間の時刻同期
     (dE/E にどのみち必要)と高周波の安定性。**ダウンスケールしたビームを空き ch に入れたテストランで確認する**(解析は Claude)。
  2. **NIM で E-OR・dE-OR・ビームの 3 重コインシデンス**(今の ThGEM × ビームと同じ形): dE・E ボードの TRG-OUT 2 本が要る
     (`out_selection = OUT_PROPAGATION_TRIGGER`、16 ch に `ch_out_propagate = TRUE`、幅 `coinc_trgout`。DT5730B の実機
     DevTree で確認、VME 版は未確認)。数珠つなぎでは最後尾の 1 枚しか TRG-OUT が空かないので、ラン信号は 1 か所から
     全ボードに配る形(例: VX2730 の GPIO を Run → FAN-OUT → 全 S-IN)にする。窓は 200 ns 未満、TRG-OUT の固定遅れを
     ビーム側の遅延で合わせる。残るのは Si と一致したパルスだけ。
  3. デジタイザ内のコインシデンス(トリガー有効化): E ボードの信号を組み合わせる論理が組めるか未確認 → 非推奨。
- ソフトでの OR(参考): イベント定義(TODO 71、`event_def` v1.1)のタグに `[モジュール, 0, 15]` で Sector をまとめ、
  トリガーにできる。L2 の counters / flags / accept で「E あり AND ビームあり」も判定できる(ビームを記録できる場合)。
- 現在の pf-qaq の ch11 = **ビーム × ThGEM のハードの同時計数**(ユーザー)。ラン 220 の約 101.5 µs 周期は
  **パルサーでの時間校正ラン**だったため(ユーザー)。
- **実データでの確認 (2026-10-29、ラン 217 = 16O+27Al、87 s、`/tmp/ch11_rf.C` on pf-qaq)**: ch11 は 9.9 kHz、100 % が
  ThGEM1 か ThGEM2 のトリガーと ±2 µs で同時 → 説明どおり。ch11 の時刻は**ビームの高周波の格子に乗っている**
  (同時計数のタイミングはビームが決めている)。周期 **T = 199.980 ns(5.000500 MHz)**。時刻間隔の位相の揃い
  R = |Σ exp(2πi·Δt/T)| / N: 4 µs 以内 0.997(≈ 2.5 ns 相当)、40 µs 以内 0.996(≈ 2.7 ns)、400 µs 以内 0.965(≈ 8.5 ns)
  (ランダムなら ≈ 0.001)。→ **案 1 は成り立つ**: 1/100 ダウンスケール(50 kHz、20 µs ごと)なら外挿は 20 µs 以内で
  ≈ 2.5 ns、1/1000 でも数 ns でバンチ(200 ns 間隔)を取り違えない。この揺らぎには ThGEM・同時計数回路の分も入っているので、
  ビームを直接記録すればもっと小さい可能性がある。

## 4. 要確認(ユーザー)

回答済み(2026-10-22): 日程 = 11 月中旬、`experiment_name` = `N_Wall`(変わることもある)、
Si = V1725 PHA × 6 台(今は module 0 ch 1 の 1 ch)、Si と中性子の時間相関あり(結線・設定はユーザー)、
ThGEM は使わず root_sink は別設定(本格 EB は後回し)、HV = MHV-4 × 1(読み出しのみ)と SY5527(IP 不明)。

回答済み(同日追加): V1725 は **8 台**(128 ch)で Si 122 ch が収まる。光リンクの振り分けは
TOML の番号を合わせるだけなので問題なし。V1730 を使うかは未定。Si は E / dE それぞれ 61 ch(計 122 ch)。

残り:

1. ~~チャンネル対応表と検出器名~~ → **実験前には決まらない**(ユーザー: いつも実験後に解析しながら正解の対応表を作る)。
   Grafana は `Mod, Ch` で始め、名前は後から付ける。やり方は TODO 74 §3.1
2. 2 本目の「細かいもの」の中身 → **完全に未定**(ユーザー)。決まった時点でこの TODO に足す

## 5. N_Wall の立ち上げ(2026-11-04〜)

### 5.1 決まったこと(ユーザー、2026-11-04)
- ラン番号はそのまま続ける(N_Wall のラン 1〜4 は VX2730 だけの試験。本番はラン 5 から)。
- 液シン(VX2730 ch8〜31)の設定は済み。**LaBr3(ch1〜7)はまだ**(今は液シンと同じ初期値)。
- **V1725 では波形を録らない**(バッファの制限でイベントを落としすぎる)。
- 11/4 にクロックの結線以外をすべてつなぐ。クロックとスタートの同期は、ケーブルが来てから。
- Si のバイアスは MHV-4 のまま行くか、CAEN 製の電源に替えるか未定。

### 5.2 用意したもの(2026-11-04)
| もの | 場所 | 中身 |
|---|---|---|
| N_Wall の TOML | `config/config_nwall_pfqaq.toml` | 6 台(module 0 = VX2730、1〜5 = V1725)、nwall_sink、`/data/N_Wall`、eLog `N_Wall`。**今は全ボード独立スタート**、同期への切り替え手順はヘッダー |
| VX2730 | `config/digitizers/nwall_vx2730_psd2_0.json` | pf-qaq でラン 4 まで使っていた調整済みの設定そのまま(id と名前だけ変更) |
| V1725 × 5 | `config/digitizers/nwall_v1725_pha_{1..5}.json` | Si の初期値: 台形 2000/1000 ns、極零 50 µs、閾値 100、RC-CR2 平滑 16、粗ゲイン ×1、record 7000 ns(< 7168)、波形 OFF。**極性は推測**(リング NEGATIVE、Sector POSITIVE) |
| eLog | `docker/elog/elogd_thgem.cfg` | `[N_Wall]` ログブックを追加(属性は ThGEM と同じ) |
| Grafana | `tools/grafana/gen_dashboards.py --config config/config_nwall_pfqaq.toml` | 6 台・112 ch のレート用ダッシュボード(手元で生成を確認) |

- V1725 の光リンクとノード(ユーザー、2026-11-05。`caen-toolbox dig1 check` で確認、全台 DPP-PHA、ROC 04.25):

  | module | link | node | 型 | PID | AMC |
  |---|---|---|---|---|---|
  | 1 | 1 | 0 | V1725 | 184 | 139.10 b5614 |
  | 2 | 1 | 1 | V1725 | 170 | 139.10 b5614 |
  | 3 | 1 | 2 | V1725 | 147 | 139.10 b5614 |
  | 4 | 0 | 0 | V1725 | 208 | 139.10 b5614 |
  | 5 | 0 | 1 | **V1725S** | 27647 | **139.137 b7110** |

  最初の読み取り(09:58)では link 1 が応答しなかった。原因は**ファイバーの R/T が逆**(ユーザー)。空のリンクと同じ速さ(約 0.25 秒)で失敗したので、
  光が届いていないと分かった(生きている数珠つなぎの空きノードは約 0.9 秒待ってから失敗する)。
  役割は対応表のまま(1 = dE Sector、2〜4 = dE リング、5 = E Sector)。
- 6 台の JSON は DELILA の読み込み処理を通ることを確認済み(`Config::load` → `to_caen_parameters`、V1725 は 35 パラメーター)。

### 5.3 配備(pf-qaq、DAQ が止まっているときに)
1. pf-qaq のローカル修正を退避して origin/master に進める(ThGEM の TOML はコメント行だけが変わるので衝突しない)。
2. 上の新しいファイルを置く。
3. Rust を `--features x743` でビルド(/api/runs/next の修正、`.delila.zst` の読み込み)。nwall_sink をビルドして `~/.local/bin` へ。
4. eLog を再起動(`docker compose restart elog`、設定ファイルは 1 ファイルの bind mount なので再起動が要る)。
5. `gen_dashboards.py` で N_Wall のレート用ダッシュボードを生成(Grafana は約 10 秒で読む)。
6. `ln -sfn config/config_nwall_pfqaq.toml config.toml` → `./scripts/start_daq.sh`。

### 5.3.1 実施記録(2026-11-04 午前、pf-qaq、ユーザーの許可を得て)
DAQ は止まったまま(全コンポーネント Configured、旧バイナリ)。**DAQ の再起動と `config.toml` の切り替えはまだ**。

| 手順 | 結果 |
|---|---|
| 1. コミット・push | fb76891(設定・eLog)、7f14ae3(この §5)を GitHub へ |
| 2. pf-qaq を更新 | 0dfde61 → **7f14ae3**(`git stash` → `merge --ff-only origin/master` → `stash pop`、衝突なし)。ローカル修正 4 つ(ThGEM の TOML の N_Wall 用の書き換え、`monitor_layout.json`、`thgem_v1730_psd1_0.json`、`thgem_vx2730_psd2_1.json`)はそのまま。退避前の差分は `~/pfqaq_local_mods_20261104.patch` |
| 3a. Rust | `cargo build --release --bins --features x743` 成功(49.6 s、警告 0、ログ `~/build_20261104.log`)。止まっている DAQ は旧バイナリのまま動き続けている |
| 3b. nwall_sink | ROOT 6.38.06 でビルド → `~/.local/bin/nwall_sink`(sha256 e4e2f5c2…)。単体テスト 56/56。**合成データでの通し試験**(`make_test_run.C` 2 s + `test_publisher`、http 18090): 273,824 ヒットを全部書き込み、イベント 3,939、T = 199.9500 ns、late 0、display_drops 0、`check_events.C` **IDENTICAL** — Mac(10-30)と同じ結果。`root_sink`(ThGEM・ELIADE 用)には触っていない |
| 4a. eLog | `docker compose restart elog` → `[N_Wall]` と `[ThGEM]` の両方が開く(HTTP 200) |
| 4b. Grafana | `gen_dashboards.py --config config/config_nwall_pfqaq.toml` → フォルダ N_Wall に「Rates — N_Wall」(uid `rates-n-wall`、6 台・112 ch) |

- 注意: N_Wall のラン 1〜4 は VX2730 が module 1 だったので、このダッシュボードで 11/4 以前を見ると、液シンの ch0〜15 が dE Sector の名前で出る(ラン 5 以降は正しい)。
- 残り: V1725 がつながったら `ln -sfn config/config_nwall_pfqaq.toml config.toml && ./scripts/start_daq.sh`(→ §5.4)。

### 5.3.2 N_Wall の構成への切り替え(2026-11-05 10:09、pf-qaq、DAQ は Configured・ランなしを確認してから)
- pf-qaq = b4c0499。`config.toml` → `config/config_nwall_pfqaq.toml`、`stop_daq.sh` → `start_daq.sh`(ログ `logs/20261105_100913/`)。
  新しいバイナリで全 9 コンポーネントと nwall_sink が起動。`/api/runs/next` = 5。
- **Configure(10:18、ランは始めていない): 6 台とも成功。** VX2730 は 53/53。V1725 は 5 台とも同じ結果で、V1725S(FW 139.137)と 139.10 の差はログに無し:
  - 丸め: `reclen` 7000 → 7008 ns(7168 未満のまま)、`ch_trap_tflat` 1000 → 1008 ns、`eventaggr` 1023 → 511。
  - `ch_trg_latency` は 5 台とも CAEN −1 で拒否(DevTree に無い。ログに WARN、Configure は成功)。
- **Monitor のレイアウトを変更**(`config/digitizers/monitor_layout.json`、pf-qaq のローカル修正のまま): VX2730 のタブ 4 つを source 1 → 0 に、
  V1730 の `PSD1-SN66` タブを削除(source 0 が VX2730 になるため)、Si のタブ 5 つ(mod1〜5、各 16 ch のエネルギー)を追加。
  変更前は `~/monitor_layout_before_nwall_20261105.json`。**ThGEM に戻すときはこれを戻す。**
- 未確認: 実データ(ランをまだ取っていない)。V1725S の時刻とエネルギー、Si の極性、eLog への投稿、Grafana のレートは最初のランで見る。

### 5.4 初日の確認
1. **各 V1725 の特定**(DAQ を止めて): `caen-toolbox dig1 check -c OPTICAL_LINK -l <link> -n <node>` で型・シリアル・
   ファームウェア(DPP-PHA)を見て、TOML のリンクとノードを合わせる。シリアルを JSON に写す。
   **V1725S(module 5)の FW 139.137 は DELILA では初めて**(確認済みは 139.10): 最初のランで Configure の結果、時刻、エネルギーを確かめる。
2. Configure が全 6 台で通る(つながっていないボードはコメントアウト。1 台でも開けないと全体が失敗する)。
3. **Tune Up で Si の極性**を見る(台形がまともに立つか)。ch15 のパルサーの極性と閾値も合わせる。
4. 極零: 台形の平らな部分が水平になるように `trap_pole_zero_ns` を合わせる(プリアンプの減衰時間)。
5. 短いランで、Grafana の「0 Hz のチャンネル」の表と nwall_sink の `/hits` で全チャンネルを確かめる。
6. LaBr3(VX2730 ch1〜7)を調整する。
7. **クロックのケーブルが来たら:** TOML のヘッダーの手順で S-IN スタートに切り替え → パルサーのランで `time_map.C`(TODO 72)→
   スタートディレイと `config/time_settings_nwall.json` → TODO 77 の T1(全ボードが同じクロックで同時に始まるか、3 ラン繰り返し)。
8. ビームが来たら: nwall_sink の較正(`si_delay_ns` と `rf_t0_ns`、`tools/root_sink/nwall/README.md`)。

### 5.5 見つかった問題(2026-11-05)
- **`GET /api/runs/:n` が実験名で絞らなかった → 修正済み(f724e75、2026-11-05 10:59 に pf-qaq へ配備):** Runs ページで N_Wall のラン 5 を開くと、詳細は X730_ThGEM_Test のラン 5、設定は N_Wall のラン 5 が混ざって出ていた。
  実行中のランへの書き込み(ノートなど)も実験名を見ていなかったので、合わせて修正。取り直しでは新しい方を返す。
  配備: pf-qaq を c105f13 に進めてビルド(48 s、警告 0)、**Operator だけ再起動**(Reader・Merger・Recorder・Monitor・nwall_sink の PID は変わらず、全台 Configured のまま)。
  確認: `/api/runs/5` = N_Wall `Time offset check`、`/api/runs/99` = 404、`/api/runs/next` = 6、`/api/runs/5/config` = 6 台。
- `X730_ThGEM_Test` のラン 29(2026-08-03)が `running` のまま残っている(side3 から移した記録)。今回の修正で他の実験から書き込まれることはなくなった。消すかはユーザー次第。
- カスケードの設定(`OUT_PROPAGATION_RUN`、`start_delay`、リングの ch15 = POSITIVE、ch0 = TTL 用)は TODO 77 §6.13 でリポジトリに取り込み済み。**VX2730 ch0 はビームを入れるときに戻す(ユーザーが行う。TTL をそのまま入れる予定、元が NIM ならそれを使う)**。
- **mod2〜4 ch15 の E = 0 は、ボードが立てた入力飽和フラグ(EXTRAS bit[4]、UM5678 p.110)と 100 % 一致(TODO 77 §6.14)。** パイルアップではない。E = 0 になる 3 台は `polarity` の既定が NEGATIVE のリングの 3 台と重なる。ラン 18(1 本の配線)では同じ設定で E = 5345。原因は未確認で、確かめ方は §6.14。
- **+17 ns のランはランごとに補正する**(ユーザー: Si は dE vs E の粒子弁別なので、きっちり合わせなくてよい)。
- **`record_length` は全ボード 20000 ns にする予定。Si を入れたあと、ユーザーが Tune Up で確認しながら変更する。** pf-qaq の mod2 は先に 20000、ほかは 7000。7168 ns を超えると Tune Up の波形では 2 チャンネルの組の同時イベントが片方だけになる(波形オフのランには影響なし)。mod2 の ch15 の個別設定は気にしない(ユーザー)。
- パルサーは、入れるならば本物の信号をそのまま出せる高級なものを使う(ユーザー)。**入れるかは未定なので、E = 0 はこれ以上追わない。**
- **Si の電源は CAEN N1408 に決定(2026-11-05、ユーザー)。モニタと ch ごとの ON/OFF・`VSET`・`ISET`・`TRIP`・`RUP`・`RDW`(10 V/s まで)の Web ページを実装、ラン中は OFF だけ(TODO 82、http://192.168.144.142:8095/)。** MHV-4 は使わない。Grafana の `channel_rate`、`run_event`、eLog への投稿(Run #41〜44 を確認)は動いている。対応表 `config/channel_map_N_Wall.txt` はまだ PROVISIONAL(Si の順番は実配線と未照合)。
- **優先はビームパルスと中性子の時間関係、つまりイベントビルドのビームパルス補完(ユーザー)。** 200 ns 刻みで考えてほぼ問題ない精度で来る。**`events` tree に `BeamTime` と周期 T を追加した(TODO 79「BeamTime」、合成データで確認、pf-qaq に配備済み。実際のビームでは未確認)。** パルスとビームの差はガンマから出す。

### 5.6 side3 の Si 設定の取り込み(2026-11-06、ユーザーの依頼)
**pf-qaq の V1725 5 台に、side3 の `Si_Test`(S7 1000 µm、run 7 まで)の設定を Operator の REST(PUT + save)で入れた。** ハードにはまだ書いていない(次の Configure か Tune Up で入る)。リポジトリには未コミットで、pf-qaq のローカル変更のまま。元の JSON と side3 の JSON は pf-qaq の `~/nwall_v1725_before_si_import_20261106/`。
side3 の設定は run 4〜7 で同じ(スナップショットで確認)。ユーザーの記憶「dE から E で変えていない」と合う。

| pf-qaq | side3 | 極性 | DC | ゲイン | しきい値 | input rise | trap rise | flat | PZ | peaking |
|---|---|---|---|---|---|---|---|---|---|---|
| mod1 dE sector、mod5 E sector | mod0 | POS | 20 | X1 | 150 | 160 | 4800 | 1200 | 50000 | 60 |
| mod2 dE ring in | mod1 | NEG | 10 | X4 | 20 | 64 | 3296 | 1200 | 50000 | 80 |
| mod3 dE ring mid | mod2 | NEG | 20 | X4 | 50 | 96 | 3296 | 1200 | 50000 | 80 |
| mod4 dE ring out | mod3 | NEG | 20 | X4 | 20 | 96 | 3296 | 1200 | 50000 | 80 |

共通: `pre_trigger` 4000、`fast_discr_smoothing` SMTH_128、`record_length` 20000。sector の 2 台には side3 mod0 の個別設定(ch9 BL 4096・PZ 10000、ch10 rise 5000)も入れた。
**取り込まなかったもの:** `start_source`、`gpio_mode`、`start_delay`(カスケードの値)、`waveforms_enabled` は false のまま、TOML の `adc_min` は 0 のまま(同期マークのため)。リングの ch15 はパルサー用に POSITIVE、X1、しきい値 100 を個別設定で残した。

**未確認(ユーザーの配線しだい):**
- リング 3 枚の割り当ては番号順に置いただけ。side3 のボードとリング位置の対応は、JSON にも Monitor のタブにもシリアルにも残っていない。
- side3 のピーク(同じ X4): mod1、mod2 は約 4800、**mod3 だけ約 1000**。信号の大きさが違うのでプリアンプごとの差とみている(未確認)。
- **side3 mod2 の ch0、ch1 は run 6、7 とも信号なし**(run 7 で 4 件と 7 件、E は 10 以下)。dE と E のどちらのテストでも同じ ch なので、検出器ではなく、使った差動 → シングルのコンバータの問題らしい(ユーザーの見立て、未確認)。同じコンバータを使うなら pf-qaq でも出ない。
- sector ch9 の上書きは本物の調整に見える(ピークは他より約 25 % 高いが FWHM は run 6 で 1.3 %、run 7 で 0.8 %。他は run 6 で 1.5〜2.8 %、run 7 で 0.9〜1.5 %)。ただ dE と E のどちらでも同じなので、検出器でなくプリアンプかボードの ch についている可能性がある。sector の 2 台とも必要かは未確認。
- **ゲイン変更が 1 ラン効かなかった形跡:** side3 mod0 は run 3 で X4 → run 4 で X1 に変えた記録だが、run 4 のエネルギーの平均は run 5 のちょうど 4.00 倍(ch0 24405 / 6099、ch9 30010 / 7500)。mod1 は変わらず、DAQ の再起動も挟んでいない。原因は未確認。今回リングは X1 → X4 なので、最初のランでピークの位置を確かめる。

### 5.7 デジタイザの配線変更(2026-11-06、ユーザー)
**§5.6 の表のボード割り当てはこの節で置き換わった。** ユーザーが配線を変更し、link / node と信号の対応を連絡。**ID と SN は変えない**(ユーザー)。SN は DAQ を止めて `caen-toolbox dig1 check` で全 link / node(0〜3 × 0〜3)を読んだ。ほかにボードはない。

| link | node | SN | ID | 信号 | 設定の出どころ | 今までの役割 |
|---|---|---|---|---|---|---|
| 0 | 0 | 170 | 2 | dE-Sector(11-07 訂正) | side3 mod0 | dE ring in |
| 1 | 0 | 184 | 1 | E-Sector(11-07 訂正) | side3 mod0 | 同じ(設定も URL も不変) |

**訂正(2026-11-07、ユーザー): mod1 = E-Sector、mod2 = dE-Sector。** 11-06 の連絡では逆に書いていた。ラン 45〜57 は逆の割り当てで録ったので、`events` ツリーは dE sector を trigger にしており、`hasDE` は E のヒットを数えている。`delila`(hits)ツリーと `.delila` は生データなので影響なし。2 枚の sector ボードの設定は SN と `start_delay` 以外同じなので、変えたのは名前だけ: `channel_map_N_Wall.txt`、`event_def_nwall.json`(E = mod1、dE = mod2)、`config_nwall_pfqaq.toml`(コメントとソース名)、`nwall_v1725_pha_{1,2}.json` の `name`、`make_test_run.C`、pf-qaq の `monitor_layout.json`(sector の 2 タブの名前と順番)、Grafana `Rates — N_Wall` 再生成(13:30 EET 確認)。マップとイベント定義は nwall_sink がラン開始ごとに読むので次のランから効く。TOML のソース名は Operator の再起動まで旧名のまま。元のファイルは pf-qaq の `~/nwall_before_sector_swap_20261107/`。
| 2 | 0 | 147 | 3 | dE-Ring3 | side3 mod3(仮) | dE ring mid |
| 2 | 1 | 208 | 4 | dE-Ring2 | side3 mod2(仮) | dE ring out |
| 2 | 2 | 27647(V1725S) | 5 | dE-Ring1 | side3 mod1(仮) | E sector |

- **Ring1/2/3 と Inner/Middle/Outer の対応は不明。** 検出器側の担当者も分からないとの回答で、そのまま使い、解析で決める(ユーザー)。`index` は Ring1 0〜14、Ring2 15〜29、Ring3 30〜44 の通し番号で、ボードをまたぐと半径の順序を意味しない。
- **信号についていくもの:** `channel_defaults`、`channel_overrides`、`record_length`、vtrace プローブ。**ボードに残すもの:** `start_source`、`gpio_mode`、`start_delay`、`io_level`、`extras`、`event_aggregation`、`coinc_trgout`、波形 OFF。リングの ch15 はパルサー用(POSITIVE、X1、しきい値 100)。旧 mod2 の ch15 だけにあった `RCCR2_SMTH_1` は SN 170 のパルサー用だったので捨てた。
- **変更したファイル:** `config_nwall_pfqaq.toml`、`channel_map_N_Wall.txt`、`event_def_nwall.json`(E = mod2、dE ring = mod3〜5)、`make_test_run.C`、pf-qaq の `nwall_v1725_pha_{2..5}.json`、`monitor_layout.json`(タブ名のみ)、Grafana の `Rates — N_Wall`(`gen_dashboards.py` で再生成)。ラン 5〜44 はパルサーのみで旧割り当てのまま。元のファイルは pf-qaq の `~/nwall_before_rewire_20261106/`。
- **S-IN / TRG-OUT のカスケードの順番は変わっていない(ユーザー)。** `start_delay` と `gpio_mode` はそのまま。
- **確認済み(2026-11-06 17:18):** Reader はそれぞれ想定の `/dev/a3818_*` を保持(source 2 = link 0、1 = link 1、3〜5 = link 2 の node 0〜2)。Configure は全台成功して Reset で Idle に戻した。ch10 の `trap_rise_time_ns` は 5000 → 5008 に丸まる(刻み 16)。`ch_trg_latency` の拒否と `eventaggr` 1023 → 511 と V1725S の CFD レジスタ読み出し失敗(-6)は変更前のログにもある。

**未確認:**
- **Ring と side3 のボードの対応は番号どおり(Ring N = side3 mod N)に置いただけで、データから決める手がかりもない。** ch0、ch1 の無信号はコンバータの問題らしく(§5.6)、見分けに使えない。ピークの約 1/5(side3 mod3)も、コンバータが原因なら同じ。違うのは `dc_offset`(10 / 20 / 20)、しきい値(20 / 50 / 20)、`input_rise_time_ns`(64 / 96 / 96)だけなので、出発点として Tune Up で Ring ごとに合わせる。
- **同じコンバータを使うと、どれかの Ring で ch0、ch1 が死ぬ。** 最初のデータで確認し、死んでいれば実験前に交換する。
- **sector の ch9、ch10 の個別設定が両方のボードに要るか。** ch9 の特徴も dE と E で同じなので、コンバータ側かもしれない。最初のデータで、ボードごとに ch9 の分解能を隣の ch と比べ、悪いボードからは外す。
- **`start_delay` はパルサーでの測定以降、Si では未確認。** リングのボードが変わったので、Si どうしの時刻は Cf で詰める予定のとおりに確認する。

### 5.8 Ring のノイズで Recorder が追いつかず、再起動でデータを失った(2026-11-06、ラン 45、46)
**ラン 45(`test`)と 46(`Co60`)は Ring のノイズで合計約 6.5 MHz。** M3 0.47 MHz、M4 3.2 MHz、M5 2.85 MHz、M0 3 万 Hz。M1 は 0、M2 は計 844 イベント(sector はほぼ鳴っていない)。

| 時刻(+02:00) | 起きたこと |
|---|---|
| 19:27〜19:38 | ラン 45(687 s)。受信 45.2 億。Recorder はラン中 180 万 ev/s しか書けず、キューは Stop 時に 76 GB |
| 19:39〜19:44 | ラン 46(303 s)。受信約 20 億。書き込みはまだラン 45 の続きで、キューはピーク 104 GB |
| 19:44:35 | メモリ枯渇(搭載 251 GB)。OOM killer が `nwall_sink`(RSS 103 GB)を kill。Operator は sink を監視していないので警告なし |
| 19:44〜19:49 | Stop 後は 330 万 ev/s(約 4.4 GB/min)で排出。残り 85 GB で、完了まであと約 19 分だった |
| 19:49:04〜20 | DAQ 再起動。SIGTERM のあと `start_daq.sh` が 5 秒待って `kill -9`。キューの 85 GB が消えた |

**結果:**
- ラン 45: `.delila` は 28.6 億 / 45.2 億(63 %)、62 ファイル。最後の `run0045_0061` はフッターなし。ROOT は未確定の `run_inprogress_1793986014.root`(4.2 GB、読めるかは未確認)。
- ラン 46: **`.delila` も ROOT も 0。** eLog と MongoDB の「51,913 万イベント、11.9 GB」は、カウンタがリセットされたあとのラン 45 の書き込みを数えたもので、ラン 46 のデータではない。同様にラン 45 の記録(12 億)も Stop 時点の書き込み数で、受信数ではない。

**書き込み速度(ファイルを閉じた時刻とイベント数から、入力は約 650 万/s):**

| 区間 | 入力 | `nwall_sink` | Recorder(`.delila`) |
|---|---|---|---|
| ラン 45 | あり | 動作中 | 175 万 ev/s、40 MB/s |
| ラン 45 と 46 の間 | なし | 動作中 | 317 万 ev/s、73 MB/s |
| ラン 46 | あり | 動作中 | 175 万 ev/s、40 MB/s |
| Stop 後 | なし | 停止 | 323 万 ev/s、74 MB/s |

`nwall_sink`(ROOT)は約 106 万 hits/s(10 秒あたりの中央値 1,061 万)。以前の測定 248 万/s(2 ファイル、`root_sink`)より低く、原因は未調査。ROOT と `.delila` は別プロセス、別キュー。sink の有無で Recorder の速度はほぼ変わらない(317 → 323 万)ので、ROOT が `.delila` を遅くしたのではない。Samsung の NVMe で 74 MB/s なのでディスクの限界でもなさそう。入力があると Recorder が約半分に落ちる原因は未調査。

**確認済みの動き:**
- Stop 後に書き続けるのは設計どおり(`WriterCommand` は順番に処理され、キューは unbounded)。ラン 46 の `DrainAndStart` もキューの後ろに並ぶので、ファイルは混ざらない。
- **タイマーは原因ではない。** 満了時に `operator.stop()` を呼ぶだけで Stop ボタンと同じ経路、次のランも始めない(`control-panel.component.ts`)。
- `[operator.backlog_autostop]` と `backlog_hard_limit_mb` は N_Wall の設定で未使用(TODO 68、既定で無効)。

**再現は 2026-11-07 以降(ユーザー):** 6 MHz は異常な状況なので、今回の速度だけで本番の判断はしない。再現時の目安: 入力 6.5 MHz でキューは 2 つ合わせて約 13 GB/分増え、約 17 分でメモリが尽きる。5 分のランなら、Stop 後に `.delila` は約 8 分、ROOT は約 25 分書き続ける。見るもの: レートを段階的に変えたときの速度、Recorder のスレッドごとの CPU。

**未決定(ユーザーに提案中):** ① backlog autostop の有効化(`nwall_sink` は対象外で、OOM はキュー 104 GB のときなので上限は 60 GB 程度。既定の drain タイムアウト 60 秒は短い) ② キューが残っている間の `stop_daq.sh` / `start_daq.sh` の扱い(`kill -9` で無言で失う) ③ キューが残っている間の Start と、ラン記録のイベント数。

