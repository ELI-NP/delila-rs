# TODO 85 — デジタイザの PLL lock の表示と、壊れたボードでランを止める

**Status: 🚧 pf-qaq に配備 (2026-11-10、cc02ea2、Configured でも PLL を読む修正込み)。6 台とも Idle・Configured で `locked` を確認。** 残 = Grafana のダッシュボード作り直し(§5 の 4)、本物のランでの確認、§5 の 5(任意)、Reader のプロセスがつなぎ直しで抜けられなくなる件(§5.1)。

## 1. 発端

- 2026-11-08 pf-qaq: V1725 のクロックの数珠つなぎ(mod1 → 2 → 3 → 4 → 5)の mod1 → mod2 のケーブルが接触不良(ユーザー確認 11-09)。11:52:45 に mod2〜5 が同じミリ秒で CAEN -6、3 台が 30 秒後に Error。**ラン 70 はそのまま約 5 時間続き、Operator は止めず、画面にも目立つ表示がなかった**(dE 側は未接続だったので物理データの損失はなし)。
- ユーザー(11-09): 「PLL lock という情報が取れるはずで、そいつの状態を表示させておきたい。こいつが立ってないとデジタイザが動かない」。1725 と 1730 は同じパラメータ、2730 は違う名前かも。
- 決定(ユーザー 11-09): **ラン中に PLL が外れたら止める**。作り始めてよい、配備はまだ。

## 2. 資料

| 機種 | 読むもの | 意味 | 出典 |
|---|---|---|---|
| DIG1(x725/x730、PSD1/PHA1) | `/par/plllocked` | "PLL locked"、TRUE/FALSE | DevTree `docs/devtree_examples/dt5730b_pha1_sn990.json` |
| 同上(元のレジスタ) | 0x8104 bit 7 PLL Unlock Detect | 0 = 前回の読み出し以降に unlock があった(残る型)。数珠つなぎの初期化中に一時的に外れることがある、とも書いてある | UM5678 rev5 p.45(`legacy/WEB_UM5678_725-730_DPP_PHA_Registers_rev5.pdf`) |
| 同上 | 0x8104 bit 8 Board Ready / 0x8178 bit 4 PLL Lock Loss | 参考(今回は使わない) | UM5678 rev5 p.45 / p.64 |
| DIG2 RAW(x27xx、PSD2/PHA2) | `/par/errorflags` | bit 2 si5341_unlock、3 si5395_unlock、4 LMK04832_unlock、5 jesd_unlock、15 clkin_miss | x2730 DPP-PSD Supported Parameters、CUP 2024092000(`legacy/documentation_2024092000-2/a00103.html`) |

- DIG2 には `plllocked` という名前はない(実機の DevTree 3 つで確認)。`/par/boardready` は「errorflags が 0」のまとめ。
- bit 15 clkin_miss は、ボードが外部クロックを使うとき(`/par/clocksource` が `Internal` 以外)だけ外れとして数える。内部クロックで CLK-IN が空なのは正常のため。
- 11 番以降の温度系のビットは時計とは関係ないので数えない。

## 3. 作ったもの

**Reader(`src/reader/pll.rs`、`read_loop_dig1.rs`):**
- RAW の read loop(PSD1/PHA1/PSD2/PHA2)が **2 秒おき**に PLL を読む。**Idle・Configured・Armed・Running のどれでも読む**(`PllPoller`)。初版も設計はそのつもりだったが、読み取りがループの「ラン中だけ」の分かれ目より後ろにあり、実際はラン中しか読んでいなかった。2026-11-10 の pf-qaq の再起動で mod2・3・5 がつながらないのに UI に何も出ず判明、ユーザーの要望で修正(テスト +3)。値は `ComponentMetrics.pll` = `locked` / `unlocked` / `unreadable`(読めない)/ なし(その FW に PLL の情報がない)。
- 接続していないときは `unreadable`。DevTree に該当パラメータがない機種は、つながった時点でわかるので「なし」にする。
- 状態が変わった瞬間に 1 回ログを出す: locked は info、それ以外は warn(silent failure を作らない)。
- 読み出しは read loop のスレッド内の ReadData の合間にやる(DIG2 のトリガーカウンタの読み出しと同じ形)。ハンドルを別スレッドから触らない。
- AMax(`read_loop_dig2.rs`)はまだ読まない(なし)。

**Operator(`src/operator/digitizer_guard.rs`、常に有効):**
- ラン中(Tune Up 以外)、ソースが **PLL `unlocked` / `unreadable`、または状態 Error** で、同じソースで **10 秒続いたら**、drain-first stop(ディスク不足と同じ。生きているボードのデータは書き切る)。
- 10 秒待つ理由: DIG1 の flag は残る型で、資料にも初期化中の一時的な外れの記述がある。光リンクの一瞬の失敗(Reader は 30 秒リトライする)で止めないため。unlocked から unreadable への変化は同じ故障の続きとして数える。
- オフラインのコンポーネント(応答なし)は数えない。遅い応答を壊れたボードと取り違えないため。
- 自動停止の通知 `auto_stop` に原因 `digitizer_fault` を追加。理由の文は例えば「PHA1-V1725-dE-Ring3: clock lost (PLL not locked) for 10 s」。

**UI:**
- Component Status の Readers の見出しに「PLL OK」、外れがあれば赤で「PLL 2 not locked」。見出しが畳まれていても見える。外れがあると一覧が自動で開く。
- 各ボードの行に「PLL」(緑)/「PLL NOT LOCKED」/「PLL ?」(赤)。
- 自動停止の赤い枠: 「Run N stopped automatically: a digitizer stopped working」、止まった後に「クロックケーブル(CLK-IN の数珠つなぎ)と PLL LOCK LED を見る。答えなくなったボードはたいていクレートの電源入れ直しと DAQ の再起動が必要」。

**Reader の終わりの合図(EOS)と Error 表示(`src/reader/eos.rs`、2026-11-09 追加、ユーザー了承):**
- 発端: run 70 の nwall_sink の ROOT が `run_inprogress_1794128280.root` のまま残った(手で `run0070_0000_N_Wall.root` に改名、中身は無傷)。nwall_sink は「そのランでデータを送ったソース全部の EOS」で閉じるが、mod2・3・5 が EOS を出さなかった。
- 真因: EOS は read loop の「取得を止める」処理(接続があって取得中のときだけ通る)から出る。30 秒読めないと read loop は接続を捨てて Error にするので、その後の Stop では止めるものがなく EOS が出ない。**Stop 自体は受理されていた**(Error は watch に入るだけで、コマンド側の状態は Running のまま)。
- 修正: read loop が「EOS をまだ出していないラン」を接続とは別に覚える(`OwedEos`)。Operator が止めたら(目標が Configured / Idle)**2 秒後**に Stop 信号 → EOS。
  - ラン中は出さない: Recorder はそのランの最初の EOS でファイルを閉じるので、ラン中の EOS はほかのボードのデータを落とす。
  - 2 秒待つのは、生きているボードの吐き出し(最大 1 秒)より後にするため。この EOS が Recorder のファイルを早く閉じる最初の EOS にならない。
  - 次のランの Arm が来たら待たずにすぐ出す。次のランが始まってしまったら出さずに error ログ(EOS は Reader の現在のラン番号で出るので、新しいランのファイルを閉じてしまう)。
- ついでに見つけた不具合: Error になっても GetStatus が報告する `hw_state` は Running のままで、**run 70 の間 Operator は死んだボードを Running と見ていた**(event_stall の「Running なのにイベントなし」だけが出ていた)。Error に変わるときに `hw_state` も Error にした。これで digitizer_guard の「Reader Error」も実際に効く(以前は PLL が読めないことでしか止まらなかった)。
- DIG1 RAW(`read_loop_dig1.rs`)と AMax(`read_loop_dig2.rs`)の両方。Stop 信号の送り直しの処理は 2 か所の同じコードを `eos::send_stop_signal` にまとめた。テスト +9。

**InfluxDB / Grafana:**
- `pll,module=N locked=1i|0i|-1i`(1 locked、0 外れ、-1 読めない)。
- Rates のダッシュボードの格子の下に「Digitizer clock (PLL)」(state timeline、ボードごとに 1 行、区間内の最悪値)。

**テスト:** Rust +16(common 2、pll 7、guard 6、influx 1)、全体 831 通過、clippy 警告 0。EOS 修正で +9、全体 840 通過(2026-11-09)。UI +5(120 通過)、本番ビルド OK。Grafana +1、既存 1 を拡張(38 通過)。

## 4. 未確認・注意

- **VME 版 V1725 で `/par/plllocked` が読めるか**は、配備のときに実機で見る(DT5730 の DevTree にはある。ユーザーの見立てでは 1725 と 1730 は同じ)。DevTree にない場合は「なし」になり、UI に出ない(ログに "No PLL status available" が info で出る)。
- **V1725 の取得中に `/par/plllocked` を読んで問題がないか**(ReadData の合間に 2 秒に 1 回)。
- ~~**クロックを抜いたとき**に FALSE が返るのか、ボードが答えなくなる(unreadable)のかは不明~~ → **FALSE(`unlocked`)、板は答え続ける**(2026-11-10 pf-qaq、§5.1)。どちらでも止まる作りは変えない(2026-11-08 のように答えなくなる場合もあるため)。
- ~~壊れた Reader が EOS を出さず nwall_sink の ROOT が名前付けされない~~ → §3 の EOS 修正で対処(実機未確認)。Reader のプロセスごと死んだ場合はまだ救えない(nwall_sink 側で Operator の Stop を見て閉じる案 B は見送り)。
- `hw_state = Error` の後、接続が戻らないまま Reset(目標 Idle)すると Error 表示が残る。復帰はクレートの電源入れ直しと DAQ の再起動(今までどおり)。

## 5. 配備と実機確認(ランの合間)

1. pf-qaq: `git pull` → `CAEN_PREFIX=/opt/delila-caen BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include" cargo build --release --bins --features x743`(前の版は `~/delila-rs-local/switch_daq_bins.sh` の方式で退避)。Recorder の Backlog queue 0 を確認してから DAQ 再起動。
2. Reader のログ: V1725 × 5 と VX2730 で "PLL locked"。
3. UI: Readers の見出しに「PLL OK」、行に「PLL」。
4. Grafana: `tools/grafana/gen_dashboards.py` でダッシュボードを作り直し、PLL の行が緑。
5. (任意、ユーザー判断)テストランでクロックケーブルを抜き、10〜15 秒で止まり赤い枠が出るか。抜いたボードの PLL が FALSE になるか unreadable になるかを記録。さらに: 30 秒たって Error になったボードが Operator で Error と出るか、停止の約 2 秒後にその Reader のログに "Sending the end-of-run marker (EOS)" が出て、nwall_sink の ROOT が `runNNNN_0000_N_Wall.root` に名前付けされるか。

### 5.1 配備 (2026-11-10、ユーザー「1, 2 をしましょう」)

- pf-qaq の手元の設定をそのまま c044f82 として repo に入れ(先に Operator と Monitor のメモリ上の設定がファイルと同じことを確認)、pf-qaq を ac86dcc → c044f82 に。手元の変更は `git stash` と `~/pfqaq_local_mods_20261110.patch`、12 ファイルとも内容は前と同じ。
- ビルド 45 秒・警告 0。14:34 EET、ラン 79 の後(Configured、Recorder と Merger のキュー 0、nwall_sink idle)に DAQ を再起動(`logs/20261110_143430`)。止めるときの強制終了なし。戻し方 = `~/delila-rs-local/switch_daq_bins.sh new`(それまで動いていた ac86dcc の組)+ 再起動。
- ラン 80 の Configure → Mixed: **mod2・3・5 が接続のたびに CAEN -6**(30 秒おきに再試行)。mod4 は 3 分後に自分でつながった。mod1 と VX2730 は正常。2026-11-08 のクロックケーブルと同じ形(数珠つなぎの mod1 より後ろ、光リンクをまたぐ)。5 台とも 07:49 にラン 79 を正常に止めていて、Configured の間は古い Reader がハードを触らないので、その後の故障は再起動まで見えない。ユーザーに PLL LOCK の LED と CLK ケーブルの確認を依頼。
- PLL が UI にもログにも一切出なかった → 上の §3 の修正(Configured でも読む)。
- その後: ユーザーが CLK ケーブルの緩みを直す(~15:00)→ mod2 は同じプロセスのまま 14:59 につながり設定も入った。mod3 はケーブル修正後も、クレートのリセット(前面はすべて緑)後も、同じプロセスの再試行はすべて -6。mod4 は 14:37 につながったがリセット -6・型番が空(DPP-NONE、SN 0)で設定を拒否したまま Configured 表示。
- 15:06 に DAQ を再起動(`logs/20261110_150630`)→ **6 台とも 1 回目でつながり、Configure(ラン 80)も全台正常**(リセット・設定・ADC 較正、-6 は 0。残る警告は既知の `ch_trg_latency` と mod5 = V1725S の CFD レジスタ)。
- 読み: 同じプロセスでのつなぎ直しでは抜けない状態が残っていた(ユーザーの見立ても「ソフト側」)。クレートのリセットが要ったかどうかは切り分けられていない。CAEN ライブラリのログ(`CAENDPPLayer.log`、mod4 分を `~/CAENDPPLayer_20261110_1437_mod4.log` に保存)では、FELib の open の中で板のリセットと PLL の確認があり、「PLL lock loss occurred」で open が失敗して -6 になる。**このログは全 Reader が同じ cwd の同じファイルに書くため、1 台分しか残らない。** 未調査: 失敗した open が CAEN ライブラリ内に何を残すか、Reader が自分で抜け出す方法(プロセスごと作り直す等)。
- 15:09 に cc02ea2(Configured でも PLL を読む)を配備(ビルド 44 秒・警告 0、`logs/20261110_150925`)。6 台とも 1 回目でつながり、**Idle の時点で全台 `PLL locked`**。Configure(ラン 80)の後も 6 台とも `pll = locked`、ログの PLL の変化は起動時の 1 回だけ(Configure のリセットの後に sticky な「外れ」は出なかった)。-6 は mod5 の CFD レジスタ(既知)のみ。
- **クロックを抜く試験(ユーザー、ランの外、15:11)**: 15:11:28 に Reset(Idle)、15:11:43 にクロックケーブルを外す → **mod2〜5 の 4 台が同時に `unlocked`(FALSE)**。板は答え続けた(`unreadable` にはならない、§4 の疑問の答え)。挿し直すと 15:12:09 までに全台 `locked` に戻った(mod2 は挿す途中で外れ・戻りを 2 回)。Reader のログの warn、Operator の `pll`、Grafana の「Digitizer clock (PLL)」の行(値 0)すべてに出た。ラン中の自動停止(§5 の 5 の残り)は未試験。

## 6. 後回し

- AMax(DIG2 OpenDPP)の PLL(`errorflags` が同じ意味かの確認から)。
- PLL が外れているときの Start 拒否(設定直後の一時的な外れと区別がいるので今回はしない)。
- 温度(`/ch/N/par/ch_adc_temperature`、DIG2 の `tempsens*`)の表示。
