# Current Sprint - TODO Index

**Updated:** 2026-11-06 — **Grafana: `Si HV (N1408)` を追加、ランの印から Tune Up を除外、赤はラン中だけ(TODO 74)。admin パスワードは公開の既定値のまま(ユーザー)。** **TODO 83: ADC → keV の表示を実装**(1 次式のみ、Monitor と root_sink の表示だけ、記録は生の ADC のまま)。係数は delila-rs の外で作る(学生などのデータ解析練習)、形式は 4 列の表 `config/energy_calib_<exp>.txt`。Monitor の keV 選択が軸の名前を変えるだけだった問題も解消。合成データで確認、master に取り込み、pf-qaq に配備済み(`monitor` と `nwall_sink` だけ再起動)。受け口の空の表 `config/energy_calib_nwall.txt` も有効で、学生の表は `mv` して Reload するだけ。実データでは未確認。Monitor のフィットも修正: 誤差が常に 0、keV 軸の Area のずれ、低カウントでノイズに収束する問題を直し、分解能 FWHM/Center を追加(§14)。
**前回 (2026-11-05)** — **ラン 5: カスケードの同期スタートを確認、VX2730 基準のずれ −71〜−144 ns、幅 0.1 ns、クロック共有(TODO 77 §6.8)。1 段の差は 16 ns の倍数 + 2.8 ns。FAN-OUT との比較を計画(§6.9)。ラン 6〜8 は 0.16 ns 以内で一致。自動スキャン(ラン 9〜18)で VX2730 に対する 8.00 ns の跳びが 10 本中 3 本。VX2730 が原因とした仮説は撤回(ELIADE でも出る、V1725 の Trigger Clock は 8 ns)。T で 5 台同時に測定: **TRG-OUT = Run で V1725 どうしの差が 0.03 ns 以内で一定(§6.12)**、FAN-OUT は不要の見込み。`start_delay` でパルサーの差 6.9 ns 以内(§6.13)。**mod2〜4 のエネルギー 0 はボードの入力飽和フラグ(EXTRAS bit[4])と 100 % 一致、原因は未確認(§6.14)**。パルサーを入れるかは未定なので E = 0 は追わない。優先はビームと中性子の時間関係: **`events` tree に `BeamTime`(ビームの hit そのもの)と周期 T を追加(TODO 79、pf-qaq に配備済み、実際のビームでは未確認)**。残 = +17 ns のランはランごとに補正(ユーザー判断)、Cf での詰め**。**Si の電源は CAEN N1408: 実機で挙動を測り(`CMD:OK` は成功の証拠にならない等)、モニタと ch ごとの ON/OFF・`VSET`・`ISET`・`TRIP`・`RUP`・`RDW`(10 V/s まで)の Web ページを実装・実機確認、ラン中は OFF だけ(TODO 82、:8095)**。**pf-qaq を N_Wall の 6 台構成へ切り替え、Configure 成功(TODO 73 §5.3.2)**。2026-11-04 — **N_Wall 立ち上げ(TODO 73 §5)**: 6 台の TOML・V1725 × 5 の初期設定・eLog ログブック・Grafana・初日の手順を pf-qaq に配備(7f14ae3、Rust と nwall_sink をビルド、DAQ の再起動はまだ)。2026-11-03 — **TODO 80 起票: 汎用 root_sink**(hits tree = 全ボード・全チャンネル・生の時刻・落とさない、events tree = イベントビルダー、補正後の時刻 + 補正表同梱、目標 1 MHz)。発端は pf-qaq で `module = 0` のまま空の .root が黙って作られたこと。ROOT の並列書き込みを調査・実測([docs/root_parallel_write_research.md](../docs/root_parallel_write_research.md)): 並列 TFile は無く TTree の IMT は効かない → **TTree の 2 ファイル**(pf-qaq で hits + events 2.48 M hits/s)。実データ: ラン 4 の 165 億ヒットを今の root_sink が取りこぼしなしで書いた。**TODO 81 起票**: 持ち出し時の `.delila.zst`(`/archive` は ZFS lz4 で既に約 2 倍)。
**前回 (2026-10-29)** — **N_Wall の DAQ 構成 v1 を決定(TODO 73 §3.1: VX2730 マスター + V1730(LaBr3 + ビーム 1/100)+ V1725 × 8 の 10 台、スタートディレイでケーブル差を補正)、TODO 77(同期スタート、本番構成は対応済み)、TODO 78(スタート失敗からの復帰)、TODO 79(root_sink の N_Wall 表示)を起票**。
**前回 (2026-10-22)** — **TODO 73〜76 起票: 11 月の pf-qaq 実験キャンペーン**(1 本目 = Si + 中性子(液シン 24 台、VX2730)、その後 = ThGEM + 中性子)。73 = 親(計画と準備状況)、74 = Grafana をイベントレートモニタに(pf-qaq で InfluxDB 3 の 432 ファイル上限により長期間のクエリが失敗、Overview が Fission2026 向けのまま、チャンネル別レートなし)、75 = Si の HV(Mesytec)、76 = 中性子の HV(CAEN HV クレート、既存 `sy5527_monitor.py` を汎用化。**公開リポジトリに SY5527 のパスワードが平文でコミットされている件**も記載)。いずれも型番・接続などユーザー確認待ち。
**前回 (2026-10-07 夕)** — **TODO 69 完了(Phase 1〜3)、pf-qaq 配備済み(6ed0949、DAQ 再起動済み・Idle)**: 書き込み失敗 → Operator の run guard(常時動作)が約 2 秒でソースから即時停止(ラン履歴は Error)、Recorder はファイルを最後の完全なブロックで切ってフッタで閉じ、以後書かない(未書き込みを計数)。空きが `disk_hard_free_mb`(既定 2 GiB)未満 → drain-first 停止 + Start 拒否(507)。root_sink も失敗を検知して書くのをやめ、`/WriteStatus` に表示。副産物: フッタの無いファイルの最後のブロックを読み落とす既存バグ(Rust recover / TDelila.hpp)を修正。E2E(模擬・実 ENOSPC・空き下限)合格。pf-qaq の Tune Up ファイルで check_events IDENTICAL(両面トリガー入り、NTriggers=2 が 6,051)。
**同日 (昼)** — **TODO 71 の A〜D と TODO 72 の測定ツールまで実装・コミット、pf-qaq に配備済み(1510277)**。ユーザー判断: built ツリー廃止(`thgem_events.C` で同じ形を再構成、ハッシュ一致)、Phase C は推奨案(BGO はクローバーごと 3 つ以上・別 PSD ボード → `time_map_ac` で担当と Δt を明示)。D1 = `M:C` 指定・`--module` 複数・delila2root `--time-calib`。D2 = `time_map.C`(ボード間のずれ → start_delay の提案 → パルサーで確認、AC の担当表)、¹⁴⁰Ce で ELIFANT の AC の組を 24/24 再現、ボードは 16 ns 単位で揃っていなかった(−418〜+106 ns)。pf-qaq は ssh で fast-forward(GitHub 未 push)、DAQ は Idle → 次のランの前に Configure、ラン後に `check_events`。
**同日 (夕)**: **TODO 71 Phase A 実装・検証済み**。root_sink に `--event-def`(汎用 `events` ツリー)と `--hits-tree`(hits ツリーを opt-in 化)。
**同日 (午後)**: **TODO 71 / 72 起票**。71 = root_sink 汎用イベントビルド(「まとめる層」: トリガー起点、時間順に束ねて取りこぼしなし、基準は優先順位で選ぶ(ELIFANT は Si)、窓は基準を中心、ヒットは近い基準へ重複なし、辞書不要の配列。ELIFANT-Event を調査して下敷きに)。72 = デジタイザ間の時刻オフセットの自動較正(パルサー / データ、ハードウェアの粗い調整 + ソフトの細かい調整、検討段階)。
**同日 (午前)**: **TODO 70 Phase 1 実装・pf-qaq 配備**: root_sink `--time-calib`(timeSettings.json、ラン開始ごとに読み直し、失敗時は前の表を維持して STALE)。単体テスト 307/179、run 56 の再生で XY 像が (0.000, 0.000) に、Rust `eb_offsets` と解決結果が一致。ラン 58 で実機の仕組みを確認(検出器が動いておらず、像の中心の実機確認は持ち越し)。
**前回 (2026-10-06 夕)**: **TODO 70 起票**: root_sink に (module, channel) 単位の時刻オフセット(Rust EB の timeSettings.json 形式を再利用、`aligned = raw − offset_ns`)。発端は pf-qaq 移設でケーブル長が変わり ThGEM の XY 像が (−18, −30) ns ずれたこと。マルチデジタイザの相関に必須。設計済み・未着手。
**同日 (昼)**: **TODO 69 起票**: ディスク書き込み失敗でランが止まらない(Recorder は Error ラッチのみ、root_sink は無検知、空き容量監視なし)。発端は ThGEM 新 PC pf-qaq のストレージ構築(SSD `/data` 記録 + HDD ZFS `/archive` 長期保存)。暫定対策 = `scripts/archive_sync.sh`(cron 同期 + `/data` 80 % 超で archive と一致した古いファイルを自動削除)。
**前回 (2026-09-30)**: **TODO 59 始動**: ELIADE HPGe のデジタイザパラメーター探索。Phase 2 スキャン（`pha_trap_tune --scan`）を実装、SW 台形の初期条件バグを修正（7 月の「BLR 乖離」の真因）。実 HPGe 信号と pre-trigger 上限の実機確認待ち。詳細は TODO 59 の改訂 2026-09-30。
**前回 (2026-08-25)**: **TODO 67 実装完了**: AMax の書き込みチャンネル範囲を FW に追従化(`CHANNEL_PAGES` を codegen が導出 → `apply_amax_channel_config` がクランプ)。発端は gant のビルド赤(codegen 生成物 3 本が空 stub 化)と、12august FW が **2ch・`PAGE_BASE` 0x800000→0x80000** に変わっていたこと。`num_channels` だけが手書きで FW 追従していなかったため、32ch config × 2ch FW で **ch30 の書き込みが旧 32ch マップの ch0 ページに重なる**危険があった。**gant 配備済(CHANNEL_PAGES=2 を実 RegisterFile から自動導出、DAQ 無停止)。残件(FW 世代の Magic 照合・num_channels 整理)はレベッカへ移管**。
**前回 (2026-08-04)**: **TODO 66 Phase 1 実装完了**: root_sink を 5 スレッド構成(Receiver→Sorter→Workers×N→Writer + Display)に再構築し、統合 built-event tree(`--built-tree`)を追加。検証 V0-V8 全合格(run0023 replay で内容完全一致・workers 数非依存の決定的出力・**4-fold 15,644 がオフライン正解と完全一致**・hits tree 全体時刻ソート化)。**side3 配備は凍結中(ユーザー出張、2026-08-05〜。帰還後に実施)**。背景: EB の C++ 移行決定(SPEC v0.8、oxyroot 圧縮未実装 + root_sink 実証)、マルチスレッド設計は TODO 66 §5。
**前回 (2026-07-21)**: TODO 65 全面完了+アーカイブ、TODO 掃除(24/26/47/51/65 → archive、詳細記録退避 65KB→18KB)。

このファイルは現在のスプリントの概要を示すインデックスです。
Claudeセッション開始時に必ず読み込まれます。

---

## MVP Status: ✅ 達成 (2026-03-13)

全 Goal 達成。PSD2 + PSD1 + PHA1 全 FW DAQ 稼働中。Grafana モニタリング + ELOG 自動投稿も稼働中。

## Active Tasks

| Priority | File | Status | Summary |
|----------|------|--------|---------|
| **1** | [87_monitor_rebin_choices.md](87_monitor_rebin_choices.md) | **🚧 実装・pf-qaq に配備 (2026-11-10、6289197) — 残 = ブラウザでの見え方の確認** | Monitor の rebin を ×1/×2/×4/×8 の 4 択に(ユーザー: 1〜8 は必須、128 以上は正気を疑う)。スライダは 1〜4096 を 100 px で選ぶ作りで飛びすぎ、割り切れない倍率で櫛、範囲を狭めると壊れ、Setup の既定 1024 が知らないうちに ×64 で保存されていた(pf-qaq の LS タブ)。倍率を保存し、ビン数はグラフが表示範囲から計算 |
| **1** | [86_run_timer_server_side.md](86_run_timer_server_side.md) | **✅ 実装・pf-qaq に配備・実機で確認 (2026-11-10、DAQ_Test ラン 1 で 1 分のタイマーが 60.8 秒で停止)** | ランのタイマーを Operator に持たせ、画面(ヘッダ、全ページ)は見せるだけにする。今はブラウザが 1 秒ごとに 1 引く作りで、Control のページにしか出ず、タブを隠すと遅れ、再読み込みやブラウザを閉じるとランが止まらない。時間切れは Operator の見張りが Stop と同じ止め方で止め、理由をラン記録と eLog に残す |
| **1** | [85_digitizer_pll_guard.md](85_digitizer_pll_guard.md) | **🚧 pf-qaq に配備、Configured でも PLL を読む (2026-11-10、cc02ea2) — 残 = ラン中の自動停止の確認(ユーザーが保留)、Reader のプロセスがつなぎ直しで抜けられなくなる件** | デジタイザの PLL lock を表示し、ラン中に PLL が外れる・読めない・Reader が Error のまま 10 秒続いたら drain-first stop(ユーザー決定)。発端 = 11-08 クロックケーブルの接触不良で mod2〜5 が落ち、ラン 70 が 5 時間続いた。DIG1 `/par/plllocked`(UM5678 p.45)、DIG2 `/par/errorflags` bit 2〜5・15(x2730 Supported Parameters)。UI(見出し「PLL OK」、行ごとのランプ、赤い枠)、InfluxDB `pll`、Grafana の state timeline |
| **1** | [84_high_rate_investigation.md](84_high_rate_investigation.md) | **✅ 測定完了・N_Wall 復帰済み (2026-11-07) — 残 = 自動停止の有効化、コミット、実験後の修正(ユーザー判断)** | pf-qaq の高レート調査(実機、`HiRate_Test`、ファイル削除済み)。**1 MHz は合格: 上限は ROOT 約 1.9 MHz、Recorder 約 2.7 MHz。** ボード 1 枚 5 MHz で Reader が遅れ、`nwall_sink` が late として捨てる(ROOT のみ)。Monitor の「遅れて始まる」= 高レート時の周期的な停止(タスク飢餓が最有力)。自動停止は動く(タイムアウトは要延長)。**ROOT を圧縮なしに(`compression = 0`、N_Wall で有効)** |
| **1** | [73_campaign_2026_11_pfqaq.md](73_campaign_2026_11_pfqaq.md) | **🚧 N_Wall 立ち上げ中 (2026-11-04)** | 11 月の pf-qaq 実験キャンペーンの親チケット。1 本目 = `N_Wall`(Si dE リング 45 + dE Sector 16 + E Sector 16 を V1725 PHA × 5、液シン 24 + LaBr3 7 + ビーム 1/100 を VX2730)、その後 = ThGEM + 中性子。**§5: N_Wall の TOML(6 台、今は独立スタート)・V1725 × 5 の初期設定・eLog `N_Wall`・Grafana のレート・初日の手順。pf-qaq に配備済み。2026-11-05 に 6 台の構成へ切り替え、Configure 成功(module 5 は V1725S、FW 139.137)**。残 = 最初のラン、Si の極性、LaBr3 の調整、クロック結線後の同期と時刻較正 |
| **1** | [78_operator_start_failure_recovery.md](78_operator_start_failure_recovery.md) | **🚧 (e)(b)(c)(a)(d) 実装済み・pf-qaq 配備済み (2026-10-30) — 残 = 実機確認(T1 と同じ日に)** | 10/22 の「開始と失敗の無限ループ」= 失敗時の 408 を Chrome が黙って再送(Force Reset も 5 ms 後に上書き)。**実装: 408 全廃 → 504 + ERROR ログ、Start 等の失敗で全体を Reset して Idle に巻き戻し(始まらなかった板名と S-IN の手がかりを返す)、UI は Mixed / Degraded / Error でも Stop と Force Reset を押せる + 「Start failed」の枠、Running 中にイベントが `no_event_warn_secs`(既定 5 秒)増えないソースを「No events」の枠とログで知らせる**。偽コンポーネントで 10/22 を再現するテストつき。(d) 途中のファイルは残す(ユーザー了承) |
| **1** | [79_root_sink_nwall_online.md](79_root_sink_nwall_online.md) | **🚧 nwall_sink v1 実装済み・pf-qaq に配備 (2026-11-04) — 残 = 同期マーク、実機での確認と較正** | N_Wall のオンライン表示。**実験ごとに別のプログラム**(`tools/root_sink/` を common / standard(root_sink、ThGEM と ELIADE)/ thgem / nwall に分けた)。nwall_sink = standard の記録の写し + N_Wall の表示(並べ替え後の hit と組んだイベントから): RF TOF(1/100 ビームからバンチの時計、周期はランごとに測定)、液シン PSD、dE vs E、Sector の相関、Si が選んだバンチからの中性子 TOF(液シン・γ・ビームは VX2730 に集めるので 8 ns の跳びが効かない)。役割は対応表 `config/channel_map_N_Wall.txt` の 5〜7 列から。トリガー = E Sector(dE 必須は L2)。E2E: check_events IDENTICAL、周期を真値どおりに測定 |
| **1** | [80_generic_root_sink.md](80_generic_root_sink.md) | **📋 設計中 (2026-11-03) — 方式 = TTree の 2 ファイル(調査・実測済み)。残 = 遅着ヒット、Stop 直前の端、追いつけないときの決まり、実装** | 汎用 root_sink。hits tree = 全ボード・全チャンネル・**生の時刻**・落とさない・可能ならソート(件数 = `.delila` を毎ラン確認)、events tree = イベントビルダー(**補正後の時刻 + 補正表同梱**)。目標 1 MHz、現実の上限は HDD(ユーザーの経験で約 3 MHz)。**ROOT 調査: 並列 TFile は無い、TTree の IMT は効かない、TBufferMerger は AutoSave 不可、RNTuple は TTree::Draw 不可・異常終了で読めない** → TTree・2 ファイル・ZSTD-5。pf-qaq 実測: TTree 1 スレッド 3.21 M hits/s、2 ファイル 2.48 M、RNTuple 8.8 M(並列 63 M)。ディスク予算: 1 MHz で 33〜45 MB/s、8 割は `.delila` |
| **2** | [81_delila_zst_export.md](81_delila_zst_export.md) | **✅ 実装済み (2026-11-03) — `scripts/delila_mv.sh`(mv の代わり、zstd -3、移動先から読み直して確認してから元を消す)+ Rust のツールと `TDelila.hpp` が `.delila.zst` を直接読む。残 = 実際のマウント先での最初の移動、各ホストへの配備** | データを DAQ ホストの外に出すときに `.delila.zst` に圧縮する(記録中はやらない)。`/archive` は ZFS lz4 が既定で効いていて `.delila` は約 2 倍 → ZFS は変えない。実測: `zstd -3 -T8` で 2.61 倍・752 MB/s、gzip は同じ縮みで 6 倍遅い、xz は 3.64 倍だが 17 MB/s。**波形入りは zstd -3 で 5.3〜6.6 倍**(x743 3.0 倍、xz なら 10〜11 倍だが遅い)。読む側はファイルをメモリに展開して今の読み込みに渡す(1 ファイル ≒ 1 GiB、約 1 秒) |
| **1** | [77_multiboard_start_sync.md](77_multiboard_start_sync.md) | **🚧 コード確認済み + 同期の実測 (2026-10-29) — 残 = pf-qaq で T1(VX2730 マスター → V1730 スレーブ)** | N_Wall の同期スタート。**ラン 221/222: V1730 マスター → VX2730 スレーブのパルサー Δt は −256.72 ns で一定(RMS 0.28 ns、ずれ無し、ラン間 0.004 ns)。ラン 223: `start_delay` +160 ns で Δt −160.04 ns → 向き確定(遅らせたボードの時刻が小さくなる)。ラン 225: VX2730 `rundelay` 248 で Δt −0.73 ns にそろった。ラン 226〜247(自動スキャン 22 本): **ボード間の時間差はランごとに 0 / 8 ns の不定(遅延なしでも 1/9、遅延ありで 12/18)。ラン中は一定** → N_Wall はパルサーを全ボードに入れたまま、ランごとに補正**。8 ns の原因調査はユーザー判断で保留(いつも同じように出るため)。本番構成(VX2730 マスター + V1725 スレーブ)は今のコードで対応済み。DIG2 スレーブの −6 は「S-IN が来ていない」ときだけ(ラン 122 で確認、一度バグと判断して撤回)。残る弱点 = DIG1 スレーブは S-IN が来なくても Running を報告する |
| **2** | [83_energy_calibration_display.md](83_energy_calibration_display.md) | **🚧 Phase 0〜3 実装済み、pf-qaq 配備済み、空の表で待機 (2026-11-06) — 残 = 実データでの確認、学生の表を置く** | ADC → keV を 1 次式で Monitor と root_sink の表示にだけ当てる。係数は外で作ったものを 4 列の表で受け取る。記録は変えない |
| **1** | [82_n1408_si_hv_control.md](82_n1408_si_hv_control.md) | **🚧 実装・実機確認済み、systemd で常駐 (2026-11-05) — 残 = Si での確認、本物のランでの拒否の確認** | Si の電源 CAEN N1408 のモニタと ch ごとの ON/OFF・`VSET`・`ISET`・`TRIP`・`RUP`・`RDW`(Web :8095、tpcdaq-rs と同じパスフレーズ、事故防止で認証ではない)。ラン中は OFF だけ。`MAXV`、`PDWN` はパネル。機器は `VSET` > `MAXV`、`TRIP` = 1000、100 V/s までのランプを受け付けるのでソフトで制限(ランプは 10 V/s) |
| **1** | [74_grafana_event_rate_monitor.md](74_grafana_event_rate_monitor.md) | **🚧 P1〜P5 と N1408 のダッシュボードを配備済み (2026-11-06) — 残 = admin パスワードの変更(ユーザー)、実際のランでの確認** | Grafana をイベントレートモニタに。P1 = InfluxDB のファイル上限、P2 = `channel_rate` に rate(名前は書かない)、P3/P4 = 汎用・実験別ダッシュボード、P5 = ランのアノテーション、P6 = アラート(後回し)。**チャンネル名は実験後に分かる → InfluxDB は (module, channel) のみ、名前は対応表(空白区切り `module channel name group`、C++ の解析と共用)からダッシュボードを生成して表示時に付ける(§3.1/3.2、採用)。見るのはユーザールーム + オフィス・自宅 → 匿名 Admin を Viewer に(決定)、pf-qaq の IP は固定できないがつけっぱなしで維持。要確認はすべて回答済み** |
| **1** | [75_mesytec_hv_monitor.md](75_mesytec_hv_monitor.md) | **📋 起票 (2026-10-22) — 仕様を詰めている段階、期限 11 月中旬** | Si の HV = **MHV-4 × 1(pf-qaq に USB 接続済み、FTDI `/dev/ttyUSB0`、by-id 固定パスあり)**。**読み出しのみ・DAQ とは独立プロセス**(操作は将来別 Web アプリ)。データシート p.6/9 の読み出しコマンドだけを許可リストで送る。実機の応答を確認済み(エコー → `\n\r` → 値 → プロンプト `MHV-4>`、電流は µA)。**V4.2 では `RUP`(設定値)が `ERROR!`、ON/OFF も読めない** → ON/OFF は出したい(ソフト側で状態を覚えない。LabVIEW 時代の教訓)。Mesytec に V4.2 のコマンドを問い合わせ、それまでは電圧から「HV 出力あり」を判定 |
| **1** | [76_caen_hv_crate_monitor.md](76_caen_hv_crate_monitor.md) | **🚧 実装済み・実機確認済み (2026-10-22) — 残 = systemd 常駐(sudo)** | 中性子ウォールの HV = SY5527 @192.168.144.148(A1833 × 2、NWA01〜NWB12)。CAENHVWrapper v7.5 を root なしで配置。`sy5527_monitor.py` を作り直し(**`VSet` は存在せず vset = 0 を書いていた → `V0Set`**、クレートマップからスロット、結果確認・再接続・SIGTERM)。SY5527 ダッシュボードのスロット選択と現在値の表を修正 |
| **1** | [71_generic_event_building.md](71_generic_event_building.md) | **✅ A〜D 実装・pf-qaq 配備済み (2026-10-07) / 残 = 最初のラン後の `check_events`、未 push** | root_sink 汎用イベントビルド。A = `events` + hits opt-in、B = ThGEM 再構成(built ツリーは廃止 → `thgem_events.C`)、C = タグ・AC・L2([設計案](71_phase_c_proposal.md))、D1 = `M:C`(TODO 70 Phase 2)、D2 = `time_map.C`(TODO 72)。検証 = ThGEM run 56 / run0023 / ELIFANT ¹⁴⁰Ce |
| **2** | [72_digitizer_time_offset_calibration.md](72_digitizer_time_offset_calibration.md) | **🚧 測定ツール実装済み (2026-10-07) — 残 = 実機確認(パルサー、start_delay の向き)、Operator からの自動化** | `time_map.C`: ボードのずれ(検出器から)→ start_delay / rundelay の変更案 → 次のランでパルサーが変更量どおり動いたか確認、AC の担当表。¹⁴⁰Ce で AC 24/24 一致、ボードのずれ −418〜+106 ns を検出。実機の確認は SN05(V1725 × 3)や ELIADE で |
| **1** | [70_root_sink_time_offsets.md](70_root_sink_time_offsets.md) | **✅ Phase 1 実装・pf-qaq 配備済み (2026-10-07) / 残 = 検出器稼働ランで XY 中心の実機確認、Phase 2 未着手** | root_sink に per-(module,channel) 時刻オフセット。`--time-calib FILE` / TOML `time_calib_file`、ラン開始ごとに読み直して固定(失敗時は前の表を維持して STALE)、Receiver の keep_module 直後で適用 → 並べ替え・hits/built ツリー・モニタすべて補正後の時間軸。provenance は UserInfo + `_timeSettings.json`。測定マクロ `delayline_offsets.C`。Phase 2 = `M:C` 指定 + ボード間クロック同期 |
| **1** | [69_write_failure_autostop.md](69_write_failure_autostop.md) | **✅ Phase 1〜3 実装・E2E 合格・pf-qaq 配備済み (2026-10-07、18357a9 / 6ed0949)** | ディスク書き込み失敗でランを止める。run guard(常時): 書き込み失敗 → 即時停止、空き < `disk_hard_free_mb` → drain-first 停止 + Start 拒否。Recorder は失敗時にファイルを最後の完全なブロックで封印。root_sink は検知して書くのをやめ `/WriteStatus` に表示。E2E = `scripts/write_failure_test.sh`(hook / disk / lowdisk) |
| **2** | [68_backlog_watermark.md](68_backlog_watermark.md) | **✅ 実装完了 (2026-08-25)、E2E 3 モード合格** | バックログ水位計測 + OOM 前の秩序ある停止(ポスター想定問答が発端)。`queue_bytes`/`backlog_level` を metrics 化、soft/hard 水位(TOML)、`/api/stop_drain` + opt-in autostop watcher で **drain-first stop**(source 停止→Running のまま drain→残り停止。静穏 3 poll 判定)。E2E: manual 254k/autostop 286k イベント**全数書き込み・損失ゼロ**、timeout 経路は残余計数報告。InfluxDB の非 source 欠落も修正。副産物: `with_results()` の success 上書きバグ発見・修正 |
| **1** | [67_amax_channel_pages_clamp.md](67_amax_channel_pages_clamp.md) | **✅ 実装完了 + gant 配備済 (2026-08-25) / 残件はレベッカ(FW 側)へ移管** | AMax レジスタ書き込み範囲の FW 追従化。codegen が RegisterFile から `CHANNEL_PAGES` を導出 → `apply_amax_channel_config` がループをクランプ + warn。テスト 8 本追加(lib 687 / codegen 27 全パス)。gant 破損復旧・12august 2ch FW のアドレス 16 倍シフト・FW 世代別 Magic 一覧も同ファイル |
| **1** | [64_amax_opendpp_params.md](64_amax_opendpp_params.md) | **🚧 Phase 0+A ✅済 (2026-07-16) / 残 = FWHM 実測(FW 修正版待ち)** | AMax UI に OpenDPP 標準 DevTree パラメータ追加（FW 開発者要望）。**Phase 0 実装+実機検証済**: `dc_offset` + `vga_gain` を AMax Input タブへ splice、FW OFFSET → "Offset (Trapezoid)" リネーム。gant SN52622（13july FW）で Tune Up Apply → DevTree 直読みで反映確認。**Phase A 済**: `docs/devtree_examples/vx2730_dppopen_sn52622.json`、**DPP_OPEN の ch パラメータは 10 個のみ**（channelstriggermask 無し）と確定。**残** = ChGain × trapezoid FWHM 実測（0/6/12 dB、13july FW の波形空バグ修正版待ち）、FW 開発者への確認事項（ChGain データパス位置/内部ビット幅）。Phase B/D は FW 開発者の要望次第 |
| **1** | [63_v1743_cfd_search_window.md](63_v1743_cfd_search_window.md) | **📋 OPEN (2026-07-09)** | 既存 x743 CFD テスト2件 fail (`cfd_valid=false`)。原因: `analyze()` の後方探索窓 `search_span=4·cfd_delay=16` が遅い立ち上がりパルスのゼロ交差を取り逃す (commit `e4ad305` から潜在)。修正前に実機パルスの rise/delay 比測定要。silent peak-fallback の可視化 (warn) も追加 |
| **2** | [52_refactor_sprint_2026-q2.md](52_refactor_sprint_2026-q2.md) | **📋 Phase 3 待機** | Phase 1+2 完了済 (23 項目、累計 -3716 行)。残 = Phase 3 Component Hardening: R-D3 (X743 read_loop split) / R-D5 (connection extract) / R-D11/D12 / R-P6 / R-P8 (ComponentRunner) / R-X3-post (ZMQ 境界 cost 再計測) |
| **2** | [event-builder/66_cpp_event_builder.md](event-builder/66_cpp_event_builder.md) | **🚧 Phase 1 完了 (2026-08-04)、side3 配備凍結中** | **EB の C++ 移行**(root_sink 系譜)。Phase 1 実装済み: eb_core.hpp(158 checks)+ 5 スレッド化 + `--built-tree`(統合イベント: 両面 + LaBr3、NaN=欠損、4-fold は cut)。検証 = run0023 replay 内容一致・決定性・R11/rollover/HTTP 負荷・**4-fold 15,644 完全一致**。恒久 E2E 基盤 test_publisher.cxx 追加。次 = side3 配備(出張明け)→ Phase 2(JSON 設定 + .delila リプレイ) |
| **1** | [59_eliade_trap_autotune.md](59_eliade_trap_autotune.md) ([JA](59_eliade_trap_autotune_ja.md)) | **🚧 ACTIVE (2026-10-02) — 実 HPGe で初回スキャン完了、FW 検証待ち** | ELIADE Ge 分解能 auto-tune（まずエネルギーのみ）。SN01 run 22（⁶⁰Co, 1 結晶）: **コアは 303 kHz 混入支配 → rise 3296 ns（=1/303 kHz）で −16 %、セグメントは白色支配で最適は 4 µs 超**（V1725 pre-trigger 上限 4000 ns）。セグメントは FW が SW より 12 % 悪い（未解明）。次 = ELIADE 1 時間ラン（FW 3296/1200）で FW 検証 → 来週（10-05〜）ツール開発集中週（FW 側スキャン自動化、セグメント乖離、config patch 出力）。詳細 §5.7 |
| - | [event-builder/SPECIFICATION.md](event-builder/SPECIFICATION.md) | **参照 (v0.8)** | Event Builder 仕様 — 概念部は C++ 実装の要求仕様として有効。実装言語判断の経緯は変更履歴参照 |

**休眠(ハード/外部待ち、archive 済)**:
[47 V1743 Step 4-7](archive/47_v1743_standard_mode_redesign.md)(2台目+S1 分配待ち)/
[51 PHA2 校正源テスト](archive/51_pha2_integration.md)(校正源待ち)/
[26 スケーリング](archive/26_multi_digitizer_scaling.md)・[24 L2 Filter](archive/24_l2_filter_implementation.md)(計画のみ)/
[50 Mac USB EEM](archive/50_mac_usb_eem_driver.md)(週末プロジェクト)

---

## ポスト MVP — 次のセッション候補

- **A:** Energy Calibration + PSD 表示 (GitHub #7) → **設計完了** (2026-02-19, [設計書](../docs/plans/energy_calibration_psd.md)) — Phase 1 から開始
- **B:** x743 統合 (GitHub #6) → **Active** ([TODO](archive/45_v1743_support.md), [設計書](../docs/plans/x743_integration.md))
- **C:** Online EB 統合 (Phase 4: ZmqHitSource + Pipeline) — **✅ 2026-05-25 完了** (Operator command shim + `delila_merger_replay` staging + `eb_monitor_cli` stub)。EB Monitor 本体 (SPEC § 11.4 item M) は未着手
- **G:** 設定自動生成スクリプト (3-4) + デプロイスクリプト改善 (3-5)
- **I:** FELib/Dig2 Rust 移植検討 — JSON-RPC プロトコル直接実装 (ポスト MVP)

---

## Recently Completed

2026-05 以前の詳細エントリは [archive/completed_2026H1_details.md](archive/completed_2026H1_details.md) へ退避(2026-07-21 掃除)。

| File | Completed | Summary |
|------|-----------|---------|
| [65_cpp_root_sink.md](archive/65_cpp_root_sink.md) | 2026-07-21 | **root_sink 完成+拡張一式**: C++ 並列 ROOT シンク(スカラー TTree recorder + Δt/JSROOT モニタ、merger PUB 追加購読)。拡張 = ①ファイル名 Recorder 完全一致 `run%04u_0000_<exp>.root`(exp は --exp-name > --operator(/api/status)> "data")②`--hists histograms.json` 宣言的ヒスト + `/ReloadHists` ライブ再構築 ③`[network.root_sink]` TOML セクションで start_daq.sh 自動起動(`2f65dad`)④ROOT 自動分割(MaxTreeSize)対策: 2TB 設定+発生時は Recorder 連番でパートリネーム(`efe43a4`、UAF 回避)。**副産物: merger EOS 転送バグ発見・修正(`dc0669c`)** — 実 reader は Stop ack 後に EOS 発行→旧 merger は not-Running discard で EOS を silent 破棄、root_sink がラン境界を検知不能だった(EOS を両タスクの discard から恒久除外)。検証 = Mac 単体 157 + gant エミュ E2E(2ラン+ロールオーバー強制)+ side3 パルサー run13/14 完全一致。マニュアル docs/root_sink_manual.md + operations_manual JA/EN 反映。side3 は TOML 起動で運用中 |
| [58_code_review_2026-06-10.md](archive/58_code_review_2026-06-10.md) | 2026-07-13 | **全コードベース精査 — CRITICAL 4 + HIGH 14 + MEDIUM 20 + LOW 11 全決着**。C/H は 2026-07-09 (`563ab77` `61229b9` `7221c7d`)。M/L 最終回 (2026-07-13, `2af404a` `0f5907b` `539a2c7`): **FIXED 15** = M1/M2 (reader 状態の正直化: config ロード失敗→Arm ブロック、Arm/Start 失敗で Armed/Running を主張しない+5s backoff)、M3 (transmute UB→範囲ガード)、M5 (subcounter run 開始 underflow)、M8 (SIGTERM handler)、M9 (command_task rebind — socket エラーで永久制御不能だった)、M12 (node_agent SIGTERM→5s→SIGKILL)、M13 (Apply 拒否で configure=502 / run_start=Arm 前中断)、M14 (tuneup_apply id 一致検証)、M15 (configure/arm/stop に Tune Up ガード)、M16 (Mongo 失敗を HTTP response に)、M17 (start の current_run fallback 構造)、M18 (Merger gap/restart warn)、L3/L4/L6/L7/L9。**文書化 4** = L5 (batch_id 非単調)、L8 (CaenHandle コメント訂正)、L10/L11 (CLAUDE.md 例外明文化)。**defer 5** = M4/M7 (64ch ボード導入時)、M6 (serde_ignored 小タスク)、M10/M11 (perf、実測後)。**受け入れ 2** = M20 (Stop テール例外の範疇)、L1 (R-X1 監査済み)。681 tests pass |
| [62_v1743_drop_rollover.md](archive/62_v1743_drop_rollover.md) | 2026-07-09 | **V1743 rollover 撤去・生 TDC 直接化** (`40e87bf` master push 済): run30 破損の真因 = 起動後 ~1ms の未初期化 DMA バッファ由来ゴミ TDC (~6件, `0x1B1B1B1B` バイト反復) を RolloverTracker 永続状態が恒久破損に増幅 → `timestamp_ns = (TDC&40bit)×5 + cfd` 直接化で自己回復 (run46 実機確認, dropped=0/1.19M)。運用ルール: ラン 90 分未満。診断 `X743TdcDiag` (後退ログ上限化)。ゴミ受け入れ方針を operations_manual §9 (JA/EN) に明記。敵対的レビュー 7 エージェント + 指摘 2 件反映 |
| [61_delila2root_cpp.md](archive/61_delila2root_cpp.md) | 2026-07-08 | **delila2root C++ 化 + `.delila` format v3** (`fdc0721`+`90cd4ea`): 自己記述スキーマ埋込 + 依存ゼロ単一ヘッダ `TDelila.hpp` + ROOT ネイティブ ZSTD (4.3x)。v2 後方互換。Side3 実機 end-to-end (V1743 波形付き 108k events) 検証済。TODO 56/57 を置換 |
| [60_amax_fw_selfconfig.md](archive/60_amax_fw_selfconfig.md) | 2026-06-26 | **AMax FW 自己設定化** (`9096c77`+`7656f0a`): codegen が page_base/stride/broadcast_base を RegisterFile から自動導出、`scripts/update_amax_fw.sh` 一発で codegen→build→UI→deploy。FW 更新毎の手動アドレス合わせを撤廃 |
| [55_amax_10g_fw_round2.md](archive/55_amax_10g_fw_round2.md) | 2026-05-08 (close 2026-07-09) | **AMax 10G FW Round 2** (`8786008`+`1984bd3`): 16ch + 16 digital probes + Tune Up sub-mode + register inspector。残っていた J2/J3 実機検証は FW が 6 月版に置換され陳腐化 (後継は TODO 60 経由で検証済) としてクローズ |

## Cancelled (archived)

| File | Reason |
|------|--------|
| [archive/22_amax_decoder_implementation.md](archive/22_amax_decoder_implementation.md) | FELib OpenDPP エンドポイントから直接イベント取得で十分。デコーダ不要 |
| [archive/41_start_stop_restructure.md](archive/41_start_stop_restructure.md) | DIG1 タイムスタンプリセット問題は別原因が判明。Start/Stop フロー再構成は不要 |

---

## Completed Features (Summary)

- Emulator + ZMQ pipeline
- Merger (zero-copy forwarding)
- Recorder (sorting + file format v2)
- Monitor (Web UI + REST API + ECharts histogram/waveform)
- Operator (control system + pipeline ordering)
- MongoDB統合 (Run履歴、Comment永続化、Notes logbook)
- Source Config Management (SourceType enum, config_file, RuntimeConfig)
- Metrics API + RateTracker
- PSD2 デコーダ + 実機動作確認 (VX2730, 10kHz)
- PSD1 デコーダ + DT5730B 実機検証
- PHA1 デコーダ + マルチデジタイザ統合テスト (PSD2 + PHA1, マルチマシン ZMQ)
- データ出力検証 (E2E テスト + recover validate/dump + ROOT マクロ)
- データ完全性 + パフォーマンス + 波形堅牢性修正
- HV Gain Matcher Python ツール (SY5527, Phase 1 完了)
- Rust Event Builder (L1 完了, 67 tests, 206M hits in ~5s)
- Gemini レビュー改善
- Settings UI v2 (6カテゴリ + SetInRun + Apply via ZMQ)
- Apply Digitizer Config via ZMQ (Idle/Configured/Running 全対応)
- Tune Up Mode (Waveform + ヒストグラム + パラメータ 3-panel UI)
- Channel Registration (Monitor チャンネル事前登録 + 個別チャンネル名)
- AMax Viewer (スタンドアロン GUI ツール: 2D Histogram + Waveform + パラメータ調整 + ROOT出力)
- Monitor Quick Create (デジタイザ選択で全チャンネルビュー自動生成)
- Monitor レイアウト永続化 (Operator REST API + ファイル保存, 全ブラウザ共有)
- PHA1 Settings UI修正 (FirmwareType PHA→PHA1統一 + Virtual Probe データ駆動型 + 単位修正)
- Stop コマンドタイムアウト修正 (decode_loop yield + Recorder writer std::thread 分離)
- Tune Up Apply スペクトラム混在修正 (Merger sender_task ステート対応 + 全パイプライン Stop→Start)
- パラメータバリデーション (DevTree snap_to_step + UI step属性)
- delila2root Phase 1 (POD Event + two-pointer merge, 10.4億events 2m31s, 6.9M/s)
- PHA1 Waveform Decoder 修正 (sign_extend_14bit 符号拡張 + digital probe D0/D1 マッピング修正)
- ROOT マクロ ns_per_sample 対応 (Waveform X軸 ns 表示)
- Operator デフォルトポート 9090 移行 + docker mongo-express 8083
- Tune Up ソフトウェアトリガー強制 (clone-and-modify, SyncConfig 両方上書き)
- Run Start Waveform Recording 警告 (MatDialog, DigitizerService チェック)
- Tune Up Waveform 積算表示 (FIFO バッファ + timestamp 重複検出 + ECharts replaceMerge)
- Run History ビューワー (Runs タブ, アコーディオン展開, config snapshot 表示)
- BSON config snapshot 修正 (HashMap<u8/u32> キーの string 変換 serde モジュール)
- Config snapshot 診断ログ追加 (起動時ロード数 + 空時 warn)
- Cross-Run EOS 汚染修正 (run_number タグ + stale EOS フィルタ)
- DecodeLoop 並列化 (4 Workers crossbeam, PSD2 3.2x改善, 全FW統一パス)
- A3818 ドライバパッチ v1.6.12-delila1 (バッファオーバーフロー+off-by-one+セマフォ+PCIe障害, 76デプロイ済)
- ReadLoop transient error retry (30s timeout, 10ms interval, 5MHz×6modules 10min検証, 20回リカバリ全成功)
- DIG1 ch_extras_opt コード強制 (PSD1/PHA1 の 48-bit timestamp を JSON 設定に依存せず handle.rs で保証)
- oxyroot ROOT 出力ベンチマーク (Vec events 0.79M/s, file-per-batch確定, Stop <130ms)
- EB 統一パイプライン Phase 0-3 (HitSource trait, pipeline.rs, source.rs, .delila直接入力CLI, time alignment histogram)
- Grafana モニタリング (InfluxDB v3 Core + Grafana 2ダッシュボード: DAQ Overview + Channel Rate 48ch Stat)
- `is_master` 削除 (TOML/Operator config → Reader `startmode` に一元化、3MV Start タイムアウト修正)
- PHA1 実機稼働 (VX1730B 光リンク, 全FW DAQ完成)
- ELOG 統合 (Docker + Rust クライアント + Run Stop 自動投稿)
- A3818 scheduling-while-atomic 修正 (spin_lock→mutex, 76デプロイ済)
- V1743 Standard mode 単体構成 (VX1743 SN:25, optical link, CFD soft fine time, 95 min long-run で 40-bit TDC rollover 通過確認)
- RolloverTracker 統一実装 (PSD1/PHA1 SW Fine TS + V1743 TDC を modulo 演算で共通処理, 旧 Instant-ベース TimestampTracker 完全削除)
- UI Audit 2026-Q2 (32 項目 punch-list audit + 11 commits、 Settings header 整理 / CAEN enum 文化人化 / Monitor を chips 化 / Runs detail 別 page + ISO 8601 / Apply 失敗 inline alert / NotificationService 全統一 / error snackbar Copy ボタン)

---

## Archived

| Directory/File | Contents |
|----------------|----------|
| `archive/phase1_basic_pipeline/` | 基本パイプライン設計 |
| `archive/phase1_components/` | CLIリファクタリング、CAEN FFI、Monitor、Merger |
| `archive/phase1_control_system/` | コントロールシステム設計 |
| `archive/phase2_infrastructure/` | タイムスタンプソート、Metrics API、Source設定管理 |
| `archive/phase3_psd_decoders/` | PSD2 バグフィックス、PSD1 デコーダ実装+実機検証 |
| `archive/phase4_data_verification/` | データ出力検証 (E2E テスト、recover dump、ROOT マクロ) |
| `archive/phase5_decoders_and_testing/` | PHA1 デコーダ、マルチデジタイザ統合テスト |
| `archive/phase5_audit_and_review/` | データ完全性監査、Gemini レビュー改善 |
| `archive/11_operator_web_ui.md` | Operator Web UI (Phase 6 に統合) |
| `archive/15_digitizer_implementation.md` | デジタイザ実装 Phase 1-5 |
| `archive/16_linux_migration_checklist.md` | Linux移行チェックリスト |
| `archive/23_event_builder_implementation.md` | Event Builder L1 実装 (Time Slice 方式) |
| `archive/19_settings_ui.md` | デジタイザ設定 UI Phase 6 |
| `archive/22_amax_decoder_implementation.md` | AMax デコーダ (Cancelled) |
| `archive/25_apply_digitizer_via_zmq.md` | Apply Digitizer Config via ZMQ |
| `archive/27_settings_ui_v2.md` | Settings UI v2 (Phase 1-6) |
| `archive/28_tuneup_mode.md` | Tune Up Mode 実装 |
| `archive/28_psd1_pha1_parameter_overhaul.md` | PSD1/PHA1 パラメータ整理 |
| `archive/29_channel_registration.md` | Channel Registration + チャンネル名 |
| `archive/29_psd1_waveform_debug.md` | PSD1 Waveform デバッグ |
| `archive/31_parameter_validation.md` | パラメータバリデーション (Phase 1-3) |
| `archive/32_stop_command_timeout.md` | Stop タイムアウト修正 |
| `archive/34_tuneup_software_trigger.md` | Tune Up ソフトウェアトリガー |
| `archive/35_waveform_recording_warning.md` | Waveform Recording 警告 |
| `archive/36_accumulated_waveform.md` | 積算 Waveform 表示 |
| `archive/39_cross_run_eos_fix.md` | Cross-Run EOS 汚染修正 |
| `archive/40_decode_loop_parallelization.md` | DecodeLoop 並列化 |
| `archive/41_start_stop_restructure.md` | Start/Stop フロー再構成 (Cancelled) |
| `archive/42_a3818_driver_patch.md` | A3818 ドライバパッチ v1.6.12-delila1 |
| [44_a3818_open_fix.md](archive/44_a3818_open_fix.md) | A3818 scheduling-while-atomic 修正 |
| `archive/24_l2_filter_implementation.md` | L2 Filter 計画 (休眠、2026-07-21 移動) |
| `archive/26_multi_digitizer_scaling.md` | 10+ デジタイザスケーリング計画 (休眠、2026-07-21 移動) |
| `archive/47_v1743_standard_mode_redesign.md` | V1743 Standard mode (Step 1-3 完了、4-7 ハード待ち) |
| `archive/51_pha2_integration.md` | PHA2 統合 (校正源テストのみ残) |
| `archive/65_cpp_root_sink.md` | root_sink (完了、2026-07-21) |
| `archive/completed_2026H1_details.md` | CURRENT.md から退避した 2026 前半の詳細完了記録 |

---

## Known Issues / Future Work

- **TIME_STEP_NS ハードコード:** `src/config/digitizer.rs` の `TIME_STEP_NS = 2` は DT5730 (500 MS/s) 固定。DT5725 (250 MS/s → 4 ns/sample) など別サンプリングレートのデジタイザに対応する場合、`DeviceInfo.sampling_rate_sps` から動的計算するか、`DigitizerConfig` にサンプリングレートを持たせる必要あり。

## Notes

- **MVP: ✅ 達成** (2026-03-13)
- **現在のフェーズ:** ポスト MVP — 安定運用中、改善・拡張フェーズ
- **実機確認済み:**
  - VX2730 (Serial: 52621, DPP_PSD2, 32ch, Ethernet, 172.18.4.57)
  - DT5730B (Serial: 990, DPP_PSD1/PHA1, 16ch, USB) — ライセンス 30 min 制限あり
  - 5x VX1730B (PSD1, 16ch each) + 1x VX2730 (PSD2, 32ch) on 172.18.4.76
  - VX1743 (Serial: 25, SAMLONG 12-bit, 8ch/4groups, Optical Link) on 172.18.4.147 — Standard mode + 95 min long-run 完了 (2026-04-23)
- **動作環境:** Linux (Ubuntu, Rust 1.93.0) + macOS (クロスマシン統合)

## Reference Documents

| Document | Location | Priority |
|----------|----------|----------|
| **x2730 DPP-PSD CUP Documentation** | `legacy/documentation_2024092000-2/` | ★★★ |
| FELib User Guide | `legacy/GD9764_FELib_User_Guide.pdf` | ★★ |
| Digitizer System Spec | `docs/digitizer_system_spec.md` | ★★★ |
| Event Bridge Wire Format | `docs/event_bridge_wire_format.md` | ★★ |
| Event Builder Spec | `TODO/event-builder/SPECIFICATION.md` | ★★★ |
