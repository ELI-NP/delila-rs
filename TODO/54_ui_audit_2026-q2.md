# TODO 54: UI Audit 2026-Q2 (Operator UI cleanup)

**Created:** 2026-05-07
**Status:** 🚧 **Round 1〜9 + 2 follow-ups + height-chain fix 完了 (14 commits)、 議論待ち 3 項目のみ**
**Audit doc:** [docs/ui_audit_2026-05-07.md](../docs/ui_audit_2026-05-07.md) (32 項目、 ユーザ判定 inline 記入済)

## 経緯

PHA2 firmware mismatch hard-fail 実装中 (TODO 53、 commit `6911651`) に Material 3 snackbar token / HttpErrorResponse handling のバグが派生発覚し、 ついでに「UI が全体的にごちゃごちゃ」 とユーザ申告 → 全画面 audit 実施 + Round 単位で実装。

## 着地状況

| Round | Commit | 主な変更 | カバーした audit 項目 |
|---|---|---|---|
| **0 (preparation)** | `f296b55` | UI audit punch list 32 項目を `docs/ui_audit_2026-05-07.md` に書き出し、 ユーザが各項目に「やってください / 放置 / 議論しましょう」 の判定 inline 記入 | 全 32 項目 |
| **1** | `e6e8eb6` | Error snackbar に **Copy ボタン** + 6 small fixes | RN-4 / RN-1 partial (digitizer sort) / CT-1 / WF-3 / WF-6 / DG-7 |
| **2** | `3815631` | Settings/Digitizers ヘッダ整理 (4 elem に圧縮 + 不要 4 ボタンを kebab 内に) + Reset rename × 3 + 動的 tab | DG-1 / DG-2 / DG-3 (a/b/c) / DG-8 |
| **3** | `c62eff8` | CAEN enum dropdowns に friendly label + matTooltip 化、 wire value 不変 | DG-5 / DG-6 |
| **4** | `af2c04c` | Monitor: tab UI を Material 化 + Manual Setup expansion 化 + binning 2 行分割 | MN-1 / MN-2 / MN-3 |
| **5** | `cde0fe2` | Runs detail を `/runs/:id` 別ページに分離 + null vs absent config snapshot 区別 + Avg Rate / Trigger Loss 列追加 | RN-1 / RN-3 |
| **6** | `9a3f052` | NotificationService 残り 9 site 移行で UI 全体 1 通知サーフェス統一 | X-1 |
| **7** | `caa3139` | Run Number override を default collapsed + Cancel-edit の warn 撤去 | CT-3 / X-4 (部分) |
| **8** | `a4ab59f` | Apply 失敗の inline alert を Configure ボタン上に persist (snackbar が消えても誘導が残る) | X-5 |
| **F1 (follow-up)** | `5442a9d` | Monitor tabs を **Material chips** に置換 (App-shell tabs と視覚階層を分離) + Runs 一覧の日時を **ISO 8601** 化 (2026-05-07 14:30、 locale 非依存・コピペ可) | MN-1 (再調整) / RN-2 派生 |
| **F2 (follow-up)** | `baa2631` | active chip の background が saturated blue + dark text で視認性悪かった件を light-blue 100 + 左端 inset shadow に修正 | MN-1 polish |
| **height-fix** | `3c575bf` | routed page の `:host { height: 100% }` 抜けを Monitor + Control に追加、 内側を `flex: 1; min-height: 0` 化、 4×4 histogram grid が viewport を突き抜けてスクロールバー出ていた件を解消 (CSS 罠調査の派生) | (audit 範囲外、 派生 fix) |
| **9** | `e70fe3a` | TimerComponent を削除して ControlPanel に Timer 機能を内包 (Run with timer + Duration + Auto-stop の inline form、 countdown + progress display、 alarm dialog + flashing card 全部移植)、 ControlPage は layout だけに簡素化 | CT-6 |

**累計**: 14 commits、 src/app 配下 10 ファイル + docs 1 + 新 page 1 (run-detail.component.ts)、 1 ファイル削除 (timer.component.ts)、 dist/ 全コミットで再ビルド済。 ng test 68/68 pass、 ng build clean。 backend 変更なし。

## ユーザ判定で skip した項目 (audit 14 件)

| ID | 内容 | 判定理由 |
|---|---|---|
| X-2 | "Online" バッジが何の online か不明 | ユーザ「問題ないので放置」 |
| X-3 | Idle 中の "0 events / 0 eve/s" サプレス | 「Idle と明示されているので問題ない」 |
| X-6 | FW 表記揺れ (`.psd2` vs `'PSD2'`) | 「後回し」 |
| X-7 | Configure / Apply / Start / Stop の keyboard shortcut | 「ユーザにショートカット文化が薄く、 本番操作頻度低、 Tune Up は Enter Apply で十分」 |
| CT-2 | Component Status カードの inline metrics 整理 | 「問題ないので放置」 |
| CT-4 | Reset ボタンの色 | 「現状でどれも色違うので問題ない」 (実装確認: Configure primary / Force Reset 灰 / Start accent / Stop warn は意図通り) |
| CT-5 | Comment と Run Notes の共存 | 「実験中に Comment は必ず見えないとおかしい」 |
| DG-4 | Board タブ長スクロール → expansion panel 化 | 「x743 は特殊なので現状でいい」 |
| DG-9 | X743 Energy タブが他 FW と粒度違い | 「x743 はまだ使うかどうかテスト中なので変更不要」 |
| DG-10 | タブラベルに FW hint | DG-1 で firmware-badge をヘッダに残したので redundant |
| DG-11 | Board タブの section 余白 | 「今ので問題なし」 |
| MN-4 | "View Name" placeholder の検出器名が project 固有 | 「問題ないので放置」 |
| MN-5 | tab × の confirm dialog | `window.confirm` で既に実装済 |
| WF-1 | Tune Up toolbar の 15+ コントロール 1 行 | 「問題ないので後回し」 |
| WF-2 | probe checkbox 6 個常時表示 | 「触るのは知ってる人だけなので OK」 |

## 残: 議論待ち 3 件

| ID | 議題 |
|---|---|
| **EM** | Emulator 自体の存在意義 — そもそも UI から触る必要があるか、 残すなら Settings から外して開発者向け hidden flag に隔離するか。 ユーザ「そもそも Emulator っているのか？そこを後で議論しましょう。」 |
| **WF-4** | Tune Up bottom row "All" モードの param table grid (5 カテゴリ並列、 数千セル相当) — 「Settings と Tune Up Mode を行き来するのがダルい」 ので一覧性は欲しい、 が default を狭めるか議論余地あり |
| **WF-5** | Normal モード vs Tune Up モードを同じ `/waveform` URL で `@if (isTuneUp())` 分岐している件。 「ほぼ Tune Up 目的なので問題ないが議論しましょう」 |

## 次のアクション

- EM / WF-4 / WF-5 は session 中に対面議論。 結論次第で別 TODO

## 関連

- [docs/ui_audit_2026-05-07.md](../docs/ui_audit_2026-05-07.md) — audit 本体 (ユーザ判定入り)
- [TODO 53](53_firmware_mismatch_hardfail.md) — 派生発生源
- CLAUDE.md "Frontend Deployment Policy" — 全 UI commit で `dist/` 同梱遵守
