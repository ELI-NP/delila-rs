# TODO 77 — マルチボード同期スタート(DIG1 + DIG2)の確認

**Status: 🚧 コード確認済み・本番構成は対応済み (2026-10-29) — 残 = pf-qaq での T1(VX2730 マスター → V1730 スレーブ)の実機確認**

**関連:** [73_campaign_2026_11_pfqaq.md](73_campaign_2026_11_pfqaq.md) §3.2(N_Wall の時間相関) /
[72_digitizer_time_offset_calibration.md](72_digitizer_time_offset_calibration.md)(`time_map.C` でボード間のずれを測る) /
`docs/operations_manual.md` §7「Start が完了せず、開始と失敗を繰り返す場合」

---

## 1. 発端 (2026-10-29)

N_Wall(V1725 PHA × 8 + VX2730)では、**VX2730 がマスター**になり、全ボードを S-IN で同期スタートする(ユーザー)。
スタート同期の実機テストの前に、Operator と Reader のスタートの手順をコードで確認した(ユーザー指示)。

## 2. コードの確認結果

**Operator**(`src/operator/client.rs`)

- `arm_all_sync`: 全コンポーネントに Arm を送り、**全台が Armed になるのを待つ**。
- `start_all_sequential`: pipeline_order ごとに(下流から)Start を並列に送り、そのグループが Running になるのを待つ。
  デジタイザは全部 order 1 なので、マスターとスレーブに同時に Start が届く。全台は Arm 済みなので問題ない。

**Reader**(`src/reader/mod.rs` の `send_arm_command` / `send_start_command`)

| ボード | スタート設定 | Arm で | Start で |
|---|---|---|---|
| DIG1(V1725/V1730) | `START_MODE_SW`(マスター) | 何もしない(Start まで遅らせる) | `armacquisition`(= 開始。Run を出力) |
| DIG1 | `START_MODE_S_IN`(スレーブ) | `armacquisition`(S-IN 待ち) | 何もしない(「already started on Arm」) |
| DIG2(VX2730) | `SWcmd`(マスター) | `armacquisition` | `swstartacquisition`(= 開始。Run を GPIO などに出力) |
| DIG2 | `SINlevel`(スレーブ) | `armacquisition`(S-IN 待ち) | `swstartacquisition`: S-IN が来れば成功、**来なければ約 1 秒待って CAEN −6** |

**結論:** 本番の構成(VX2730 マスター + V1725 スレーブ)は、今のコードで対応済み。逆向き(DIG1 マスター + DIG2 スレーブ)も動く(§3)。

## 3. 実ログでの裏付け(pf-qaq、2026-10-22)

ランごとの設定スナップショット(MongoDB、`/api/runs/<n>/config`)と Reader のログを突き合わせた。

| ラン | V1730(マスター)の出力 | VX2730 のスタート元 | VX2730 の `swstartacquisition` |
|---|---|---|---|
| 17:08 の試行(4 回) | Run を出さない設定 | `SINlevel` | 約 1 秒待って −6「Command error」→ Running にならず失敗 |
| 121 | `OUT_PROPAGATION_SYNCIN` | `SWcmd`(両方 SW) | 成功 |
| **122** | **`OUT_PROPAGATION_RUN`** | **`SINlevel`** | **すぐに成功、同期して開始**(V1730 の開始の約 1 ms 前に返った) |

```
17:08 の試行 (logs/20261022_170246/reader_1.log)
  14:08:23.807 Starting digitizer acquisition (PSD2)
  14:08:24.808 ERROR Failed to start acquisition — staying stopped, retrying in 5 s  error=CAEN error -6
ラン 122 (logs/20261022_170919/reader_1.log)
  14:10:44.087 Starting digitizer acquisition (PSD2)
  14:10:44.088 Received START signal from digitizer
```

- 17:08 の失敗の原因は **S-IN に信号が来ていなかったこと**(ユーザー: V1730 の Run を TRG-OUT から出して VX2730 に入れれば同期スタートしていた)。
- **撤回の記録:** 最初は 17:08 のログだけを見て「DIG2 スレーブには必ず `swstartacquisition` が送られ、必ず −6 で失敗するバグ」と判断し、
  修正(S-IN スレーブでは何もしない)を計画した。ラン 122 の成功と照らし合わせて誤りと分かり、撤回した。
  その修正を入れていたら、S-IN が来ないときの大きな失敗(−6)が消え、「Running なのにイベント 0」という気づきにくい失敗に変わっていた。
  **今の −6 は「S-IN が来ていない」ことを知らせる正しい失敗なので、変えない。**

## 4. 本番の構成で残る弱点(今回は直さない)

DIG1 スレーブ(V1725)の Start は何もしないので、マスター(VX2730)の Run が届かなくても **Running を報告し、イベント 0 のまま気づきにくい**
(DIG2 スレーブの −6 とは逆)。対策案:

- (a) Operator 側: Running 中に、あるソースのイベント数が N 秒増えなければ警告する(FW に依存しない。Si は低レートなので N は長め)。
- (b) DIG1 のハードの取得状態を読む(パラメーター名と意味は CAEN の資料で要確認)。

当面は、Grafana のボード別レート(0 Hz が目立つ)と運用マニュアルの項目で補う。

## 5. テスト計画(pf-qaq、ランの合間に、ユーザーの了承と結線をもらってから)

**T1(本番と同じ向き): VX2730 マスター → V1730 スレーブ**

- 設定: VX2730 = `start_source: SWcmd`、`gpio_mode: Run`(または `trgoutmode: Run`) / V1730 = `startmode: START_MODE_S_IN`。
- 結線: VX2730 の GPIO(または TRG-OUT)→ V1730 の S-IN。共通のパルサーを両ボードの空き ch に入れる。
- 準備: pf-qaq の TOML で VX2730(source 1)を有効に戻す(今はコメントアウト)→ ランの合間に DAQ を再起動。
- 確認すること:
  1. Start が完了する(Operator のログに「Group order=1 all Running」)。
  2. 両ボードからイベントが来る(V1730 の 0 イベントは §4 の弱点なので、Grafana で特に見る)。
  3. `time_map.C`(TODO 72)でボード間のずれを測り、3 ラン繰り返してずれが一定か見る。
  4. Tune Up(SW スタートへの強制切り替え `force_software_trigger`)が両ボードで動く。

**T2(逆向き): V1730 マスター → VX2730 スレーブ** — ラン 122 で動作を確認済みなので省略してよい。

## 6. 結果

### 6.1 ラン 221 / 222(2026-10-29、pf-qaq、V1730 マスター → VX2730 S-IN スレーブ、設定は 10/22 のラン 122 と同じ)

- 構成: mod0 = V1730(`START_MODE_SW`、TRG-OUT = Run、内部クロック)、mod1 = VX2730(`SINlevel`、`clocksource = FPClkIn`)。
  同じパルサー(約 9.87 kHz)を **mod0 ch12** と **mod1 ch0** に入れた。各 約 21 秒。
- 解析: `pulser_dt.C`(`.delila` を TDelila で直接読む。ヒットの少ない ch の各ヒットに、もう一方の一番近いヒットを対応させる)。

| ラン | Δt = t(m0 ch12) − t(m1 ch0) の中央値 | RMS(±10 ns 内) | ラン中のずれ | 対応できた割合(m1 ch0 基準) |
|---|---|---|---|---|
| 221 | **−256.725 ns** | 0.281 ns | +0.0002 ns/s | 99.9 %(うち ±10 ns 外 5.3 %) |
| 222 | **−256.721 ns** | 0.282 ns | −0.0011 ns/s | 99.5 %(うち ±10 ns 外 5.1 %) |

- **同期スタートでの Δt は、ラン中も、ランを取り直しても一定**(差 0.004 ns)。ずれ(ドリフト)も無い → 時計は共有されている。
- ヒストグラムは 0.6 ns ほど離れた 2 本の山(細かい時刻の量子化と思われる。16 ns 刻みの調整には影響しない)。
- **mod0 ch12 にパルサーと無関係のヒットが約 36 kHz**(パルサーの周期の中で一様、電荷の中央値 1430 vs パルサー 7246)。
  このため、パルサーの約 5 % で mod0 側のヒットが欠ける(デッドタイムと思われる)。測定には影響しない。

### 6.2 ラン 223: mod0(V1730、マスター)の `start_delay` = 160 ns(2026-10-29)

設定は 6.1 と同じで、mod0 の `start_delay` だけ 0 → 160 ns(MongoDB の設定スナップショットで確認)。約 34 秒。

| ラン | `start_delay`(mod0) | Δt の中央値 | RMS | ラン中のずれ | 221/222 からの変化 |
|---|---|---|---|---|---|
| 221 / 222 | 0 | −256.72 ns | 0.28 ns | ≤ 0.001 ns/s | — |
| **223** | **160 ns** | **−416.766 ns** | 0.287 ns | −0.0004 ns/s | **−160.04 ns** |

- **向きが確定: ボードの `start_delay` を D 増やすと、そのボードの時刻は D 小さくなる**(CAEN の説明どおり、TODO 72 §6 の約束どおり = `sign=+1`)。
- 量も設定どおり(160 ns に対して 160.04 ns)。ラン中のずれは無く、幅も変わらない。
- **マスターに遅延を入れても、Run の出力は遅れない**(遅れるなら Δt は変わらないはず)。マスター自身の取得開始だけが遅れる。
  → 遅延はマスターにもスレーブにも入れられる。N_Wall の本番(VX2730 マスター)で VX2730 の時刻をずらしたいときの DIG2 `rundelay` は未確認。
