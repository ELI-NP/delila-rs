# TODO 85 — デジタイザの PLL lock の表示と、壊れたボードでランを止める

**Status: 🚧 実装済み・テスト通過・未配備 (2026-11-09)。** 残 = pf-qaq への配備(ランの合間、ユーザー判断)と実機確認(§5)。

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
- RAW の read loop(PSD1/PHA1/PSD2/PHA2)が **2 秒おき**に PLL を読む。接続中はいつでも読み、ラン中に限らない。値は `ComponentMetrics.pll` = `locked` / `unlocked` / `unreadable`(読めない)/ なし(その FW に PLL の情報がない)。
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

**InfluxDB / Grafana:**
- `pll,module=N locked=1i|0i|-1i`(1 locked、0 外れ、-1 読めない)。
- Rates のダッシュボードの格子の下に「Digitizer clock (PLL)」(state timeline、ボードごとに 1 行、区間内の最悪値)。

**テスト:** Rust +16(common 2、pll 7、guard 6、influx 1)、全体 831 通過、clippy 警告 0。UI +5(120 通過)、本番ビルド OK。Grafana +1、既存 1 を拡張(38 通過)。

## 4. 未確認・注意

- **VME 版 V1725 で `/par/plllocked` が読めるか**は、配備のときに実機で見る(DT5730 の DevTree にはある。ユーザーの見立てでは 1725 と 1730 は同じ)。DevTree にない場合は「なし」になり、UI に出ない(ログに "No PLL status available" が info で出る)。
- **V1725 の取得中に `/par/plllocked` を読んで問題がないか**(ReadData の合間に 2 秒に 1 回)。
- **クロックを抜いたとき**に FALSE が返るのか、ボードが答えなくなる(unreadable)のかは不明。どちらでも止まる作りにしてある。
- 壊れた Reader はランの終わりの合図(EOS)を出さないので、自動停止の後も **nwall_sink の ROOT が `run_inprogress_*.root` のまま名前付けされない**可能性がある(ラン 65 と同じ)。.delila は Recorder が Stop で閉じる。nwall_sink 側の課題として別に扱う。

## 5. 配備と実機確認(ランの合間)

1. pf-qaq: `git pull` → `CAEN_PREFIX=/opt/delila-caen BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include" cargo build --release --bins --features x743`(前の版は `~/delila-rs-local/switch_daq_bins.sh` の方式で退避)。Recorder の Backlog queue 0 を確認してから DAQ 再起動。
2. Reader のログ: V1725 × 5 と VX2730 で "PLL locked"。
3. UI: Readers の見出しに「PLL OK」、行に「PLL」。
4. Grafana: `tools/grafana/gen_dashboards.py` でダッシュボードを作り直し、PLL の行が緑。
5. (任意、ユーザー判断)テストランでクロックケーブルを抜き、10〜15 秒で止まり赤い枠が出るか。抜いたボードの PLL が FALSE になるか unreadable になるかを記録。

## 6. 後回し

- AMax(DIG2 OpenDPP)の PLL(`errorflags` が同じ意味かの確認から)。
- PLL が外れているときの Start 拒否(設定直後の一時的な外れと区別がいるので今回はしない)。
- 温度(`/ch/N/par/ch_adc_temperature`、DIG2 の `tempsens*`)の表示。
