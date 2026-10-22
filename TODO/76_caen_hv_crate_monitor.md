# TODO 76 — 中性子検出器の HV(CAEN HV クレート)を読み出して Grafana に出す

**Status: 🚧 実装済み・pf-qaq で実機確認済み (2026-10-22) — 残 = systemd への常駐登録(sudo、ユーザー)、TODO 74 P4 の実験別ダッシュボードへの HV パネル**

**関連:** [73_campaign_2026_11_pfqaq.md](73_campaign_2026_11_pfqaq.md)(親) /
[74_grafana_event_rate_monitor.md](74_grafana_event_rate_monitor.md) /
[75_mesytec_hv_monitor.md](75_mesytec_hv_monitor.md)(同じ形の MHV-4 モニタ) /
[tools/hv_monitor/sy5527_monitor.py](../tools/hv_monitor/sy5527_monitor.py) /
`docker/grafana/provisioning/dashboards/json/hv_{monitor,vmon,imon}.json`

---

## 1. 発端 (2026-10-22)

中性子検出器(液シン 24 台、VX2730)の HV を **CAEN の HV クレート**から読み出し、
InfluxDB 経由で Grafana に出したい(ユーザー)。

## 2. クレート(2026-10-22 ユーザー回答 + 実機で確認)

- **SY5527**、**192.168.144.148**(pf-qaq と同じ実験場のネットワーク。直結やスイッチは不要)、ユーザー `admin`。
  パスワードは誰もが最初に試す初期値で、**漏れても問題ない**(ユーザー)→ 設定ファイルにそのまま書く。
- 読み出し専用で接続して確認(`InitSystem` → `GetCrateMap` → `GetChParam` / `GetChName` → `DeinitSystem`):
  スロット数 6、**slot 1 と slot 3 に A1833**(12 ch、負、3 kV / 3 mA、SN 1105 / 1092、FW 5.00)。
  チャンネル名 **`NWA01`〜`NWA12`(slot 1)、`NWB01`〜`NWB12`(slot 3)** = 中性子ウォール 24 台。
  確認時は全 ch ON(1175〜1800 V、約 250〜385 µA、Status は ON のみ)。
- `IMon` の単位は µA(パラメーター属性 Unit = A、Exp = −6)。

## 3. CAENHVWrapper

- **v7.5**(2026-09)。CAEN のサイトはログインが要るので、**ダウンロードはユーザー**(`~/Downloads` → pf-qaq の
  `~/delila-rs/external/CAENHVWrapper-v7.5.tgz`、`external/*.tgz` は git 管理外)。
- 中身を確認してから使用: `install.sh` は root で `/usr/local/lib` に入れて ldconfig するだけ(ネットワーク不要)。
  依存は libc / libm / libpthread / libdl のみ。
- **pf-qaq には root なしで配置**: `~/.local/opt/CAENHVWrapper-v7.5`、
  `~/.local/lib/libcaenhvwrapper.so` → `bin/x86_64/libcaenhvwrapper.so.v7.5`。
  システムに入れたくなったら `sudo ./install.sh` で `/usr/local/lib/libcaenhvwrapper.so`(設定の `lib_path` を変える)。
- 使う定数はヘッダー(v7.5)で確認: `SY5527 = 3`、`LINKTYPE_TCPIP = 0`、`MAX_CH_NAME = 12`。

## 4. `sy5527_monitor.py` の作り直し(2026-10-22)

実機で見つかった既存の問題と対処:

| 問題 | 対処 |
|---|---|
| **設定電圧を `VSet` で読んでいた。ボードにこのパラメーターはなく −5「Parameter not present」、結果を見ていないので vset = 0 を書いていた**(Fission2026 でもそうだったはず) | 標準の **`V0Set`** を読む。テストで `VSet` に戻らないよう固定 |
| 1 スロット 12 ch 固定、スロットは Fission の一覧で固定 | **`GetCrateMap` から実際のスロットとチャンネル数**を取る。設定に `slots` があればその中に絞る(Fission の設定はそのまま動く) |
| `VMon` 以外の読み出し結果を確認しない | すべて確認し、失敗した項目は**書かない**(0 を書かない)。警告は同じ内容なら 5 分に 1 回 |
| 通信が切れても再接続しない | 何も読めなくなったら切断 → 10 秒後に再接続(クレートの構成も読み直す) |
| systemd の停止で `DeinitSystem` しない | SIGTERM / SIGINT で切断してから終了 |
| タグのエスケープが空白だけ | `,` `=` 空白をエスケープ |

- 出力は従来と同じ `sy5527_hv,slot,channel,name` + `vmon,vset,imon,status,pw`(既存ダッシュボードはそのまま使える)。
- **書き込み系の関数は一切使わない**(`CAENHV_Set*` が無いことをテストで確認)。
- テスト `test_sy5527_monitor.py` 13 本(MHV-4 と合わせて 52 本、Mac と pf-qaq で OK)。
- 設定 `sy5527_config_pfqaq.yaml`(`slots` なし = 全スロット、5 秒周期)、systemd unit `sy5527-monitor.service`(DAQ とは独立)。
- **実機テスト**(pf-qaq、17 秒): 警告 0、24 ch × 3 周期、vset は実際の設定値(0 は 0 件)、SIGTERM で `DeinitSystem` rc = 0。

## 5. ダッシュボード

- 「HV Monitor (SY5527)」(`hv_monitor.json`): スロットの選択肢が Fission の 10 スロットに固定されていて、
  「All」でも **slot 3(NWB01〜12)が出なかった** → 0〜15 に。現在値の表は時間の範囲なし + `LIMIT 120`
  (Fission の 10 スロット × 12 ch 前提)で、24 ch だと古い値が混ざった → **直近 1 分の各チャンネルの最新値**に。
  実データで 24 行(NWA01〜NWB12)を確認。
- 「HV VMon」「HV IMon」は Fission2026 のスロット・検出器名(GL / NL)専用の作り。pf-qaq では使わない(変更なし)。
- 中性子ウォールの HV は TODO 74 P4 の「Si + 中性子」ダッシュボードにも置く(未)。

## 6. 要確認(ユーザー)

1. ~~型番~~ → SY5527 / ~~IP~~ → 192.168.144.148 / ~~パスワードの扱い~~ → 問題なし(回答済み)
2. 読み出しだけでよいか(電圧の設定はクレート側 / GECO で行う前提)— 実装は読み出しのみ
3. **常駐登録(sudo)**:
   `sudo cp ~/delila-rs/tools/hv_monitor/sy5527-monitor.service /etc/systemd/system/ && sudo systemctl daemon-reload && sudo systemctl enable --now sy5527-monitor`
