# TODO #44: A3818 `a3818_open()` scheduling-while-atomic 修正 + Reader 再接続バックオフ

**Status: ✅ COMPLETED (2026-03-05, 76 デプロイ済)**
**Created:** 2026-03-05
**Priority:** CRITICAL — 本番フリーズの直接原因

## Summary

A3818 ドライバの `a3818_open()` が `spin_lock(&CardLock)` 保持中に `msleep()` を呼び、
カーネル `BUG: scheduling while atomic` を引き起こす。12 Reader の同時再接続で増幅され
システムフリーズに至る。

## Background

- **2026-03-04 17:54:** 172.18.4.76 フリーズ (Run 192, Fission2026)
  - 15:52:02 UTC: A3818 光リンク障害 → 10/12 Reader が同時に CAEN error -6
  - 30s timeout → Error 遷移 → 1s 間隔再接続ストーム → BUG → フリーズ
- **2026-03-05 10:33:** DAQ 再起動でも BUG 再発 + segfault
- **解析:** [docs/a3818_driver_analysis.md](../docs/a3818_driver_analysis.md) Bug 7

## Analysis

### ドライバ側 (a3818.c line 784-790)
```c
spin_lock( &s->CardLock);                  // preempt_count = 2
if (a3818_reset_onopen(s) == A3818_OK)     // msleep(10) + msleep(1) inside!
    s->GTPReset = 1;
spin_unlock( &s->CardLock);
```

問題:
1. `spin_lock` 保持中の `msleep` → `BUG: scheduling while atomic`
2. `spin_lock` (IRQ 無効化なし) → 同一 CPU IRQ デッドロックリスク
3. `GTPReset` チェックがロック外 → 複数プロセスが同時にリセット実行

### Reader 側 (src/reader/mod.rs)
```rust
const RECONNECT_COOLDOWN: Duration = Duration::from_millis(1000);  // 固定 1s
```

問題:
- 12 Reader が同時に Error → 全て 1s 後に同時 `a3818_open()` → Thundering Herd

## Fix Plan

### Phase 1: ドライバ修正 (v1.6.12-delila2)

| File | Change |
|------|--------|
| `a3818.h` | `struct mutex GTPResetMutex` を `a3818_state` に追加 |
| `a3818.c` | `a3818_open()`: `spin_lock` → `mutex_lock(&GTPResetMutex)` + double-checked locking |
| `a3818.c` | `a3818_init_board()`: `mutex_init(&s->GTPResetMutex)` 追加 |

### Phase 2: Reader 再接続バックオフ

| File | Change |
|------|--------|
| `src/reader/mod.rs` | `RECONNECT_COOLDOWN` → 指数バックオフ (1s→2s→4s→8s→16s→max 30s) + ジッター (±500ms) |

## Verification

```bash
# ドライバビルド + デプロイ (172.18.4.76)
make clean && make && sudo rmmod a3818 && sudo insmod src/a3818.ko

# DAQ 起動
./scripts/start_daq.sh config/config_76_full.toml

# BUG チェック
sudo dmesg -T | grep -E 'BUG|scheduling while atomic'

# 光リンクエラーシミュレート (ケーブル抜差し) → バックオフで安全に再接続確認
```

## Gemini Review Notes

- Option A (専用 Mutex) 推奨 — CardLock は IRQ ハンドラと共有なので mutex に置換不可
- Option D (mdelay) 不採用 — 10ms ビジーウェイトは DMA 割り込みを逃すリスク
- ISR 安全性: GTP リセット中は割り込み発生せず、ISR は 0xFFFFFFFF チェックで skip → 安全
- Reader 側: 指数バックオフ + ジッターで Thundering Herd 問題を解消
